use crate::config::ServerConfig;
use crate::metrics::ServerMetrics;
use nova_core::document::{Document, DocumentId};
use nova_core::error::{ErrorCode, NovaError, Result};
use nova_core::event::{DataEvent, EventType};
use nova_core::value::Value;
use nova_index::{HashIndex, IndexManager, IndexType, OrderedIndex};
use nova_protocol::ResponsePayload;
use nova_query::ast::{Assignment, Expr, SortClause, SortDirection, Statement};
use nova_query::evaluator::Evaluator;
use nova_query::planner::{QueryPlan, QueryPlanner};
use nova_storage::format::{NbfRecord, RecordType};
use nova_storage::recovery::{CollectionKey, RecoveryManager};
use nova_storage::wal::WriteAheadLog;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::info;

/// In-memory collection storage: DocumentId -> Document
pub type DocumentStore = HashMap<DocumentId, Document>;

/// Core Database Engine managing storage, concurrency, indices, and event streaming.
pub struct DatabaseEngine {
    pub config: ServerConfig,
    collections: RwLock<HashMap<CollectionKey, DocumentStore>>,
    indexes: RwLock<HashMap<CollectionKey, IndexManager>>,
    wal: Arc<WriteAheadLog>,
    event_tx: broadcast::Sender<DataEvent>,
    pub metrics: Arc<ServerMetrics>,
}

impl DatabaseEngine {
    /// Initialize database engine, run recovery, and rebuild indices.
    pub fn open(config: ServerConfig, metrics: Arc<ServerMetrics>) -> Result<Arc<Self>> {
        let wal_dir = config.data_dir.join("wal");
        let sync_mode = config.get_sync_mode();

        // 1. Run crash recovery over WAL directory
        let recovered = RecoveryManager::recover(&wal_dir)?;

        // 2. Open WAL commit log
        let wal = WriteAheadLog::open(&wal_dir, sync_mode, 16 * 1024 * 1024)?;
        wal.set_sequence(recovered.max_sequence);

        let (event_tx, _) = broadcast::channel(4096);

        let mut collections = HashMap::new();
        let mut indexes = HashMap::new();

        for (key, doc_map) in recovered.collections {
            let mut mgr = IndexManager::new();
            // Rebuild default primary key / field indices if needed
            let _ = mgr.rebuild(doc_map.values());
            collections.insert(key.clone(), doc_map);
            indexes.insert(key, mgr);
        }

        info!(
            recovered_collections = collections.len(),
            sequence = recovered.max_sequence,
            "Database engine initialized"
        );

        Ok(Arc::new(Self {
            config,
            collections: RwLock::new(collections),
            indexes: RwLock::new(indexes),
            wal: Arc::new(wal),
            event_tx,
            metrics,
        }))
    }

    /// Subscribe to the live data flow broadcast channel.
    pub fn subscribe_events(&self) -> broadcast::Receiver<DataEvent> {
        self.event_tx.subscribe()
    }

    /// Publish an event to all active WATCH subscribers.
    fn emit_event(&self, event: DataEvent) {
        self.metrics.inc_events();
        let _ = self.event_tx.send(event);
    }

    /// Execute a parsed NQL statement within a namespace (default: "default").
    pub async fn execute(&self, stmt: Statement, namespace: &str) -> Result<ResponsePayload> {
        self.metrics.inc_queries();

        match stmt {
            Statement::Find {
                collection,
                filter,
                sort,
                limit,
                offset,
            } => {
                self.metrics.inc_reads();
                self.execute_find(namespace, &collection, filter, sort, limit, offset)
                    .await
            }
            Statement::Insert {
                collection,
                document,
            } => {
                self.metrics.inc_writes();
                self.execute_insert(namespace, &collection, document).await
            }
            Statement::Update {
                collection,
                assignments,
                filter,
            } => {
                self.metrics.inc_writes();
                self.execute_update(namespace, &collection, assignments, filter)
                    .await
            }
            Statement::Remove { collection, filter } => {
                self.metrics.inc_writes();
                self.execute_remove(namespace, &collection, filter).await
            }
            Statement::Count { collection, filter } => {
                self.metrics.inc_reads();
                self.execute_count(namespace, &collection, filter).await
            }
            Statement::Exists { collection, filter } => {
                self.metrics.inc_reads();
                self.execute_exists(namespace, &collection, filter).await
            }
            Statement::CreateIndex {
                collection,
                field,
                index_type,
            } => {
                self.execute_create_index(namespace, &collection, &field, index_type)
                    .await
            }
            Statement::DropIndex { collection, field } => {
                self.execute_drop_index(namespace, &collection, &field)
                    .await
            }
            Statement::Watch { .. } => {
                // Handled specifically in connection pipeline
                Err(NovaError::invalid_query(
                    "WATCH should be routed to stream handler",
                ))
            }
            Statement::Begin | Statement::Commit | Statement::Rollback => {
                Ok(ResponsePayload::Success {
                    message: "Transaction statement accepted".to_string(),
                    affected: 0,
                })
            }
        }
    }

    async fn execute_insert(&self, ns: &str, coll: &str, doc: Document) -> Result<ResponsePayload> {
        let key = (ns.to_string(), coll.to_string());

        // Validate document ID uniqueness
        {
            let colls = self.collections.read().await;
            if let Some(store) = colls.get(&key) {
                if store.contains_key(&doc.id) {
                    return Err(NovaError::new(
                        ErrorCode::DocumentAlreadyExists,
                        format!("Document '{}' already exists in '{coll}'", doc.id),
                    ));
                }
            }
        }

        // 1. Serialize document and append to WAL
        let payload = NbfRecord::payload_from_document(&doc)?;
        let seq = self
            .wal
            .append(RecordType::Insert, ns, coll, doc.id.clone(), payload, None)?;

        // 2. Update in-memory collection
        {
            let mut colls = self.collections.write().await;
            let store = colls.entry(key.clone()).or_default();
            store.insert(doc.id.clone(), doc.clone());
        }

        // 3. Update secondary indexes
        {
            let mut indexes = self.indexes.write().await;
            let idx_mgr = indexes.entry(key).or_default();
            let _ = idx_mgr.on_insert(&doc);
        }

        // 4. Emit Data Flow Event
        self.emit_event(DataEvent::new(
            seq,
            EventType::Create,
            ns,
            coll,
            doc.id.clone(),
            None,
            Some(doc.clone()),
            None,
        ));

        Ok(ResponsePayload::Records {
            documents: vec![doc],
            count: 1,
        })
    }

    async fn execute_find(
        &self,
        ns: &str,
        coll: &str,
        filter: Option<Expr>,
        sort: Option<SortClause>,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<ResponsePayload> {
        let key = (ns.to_string(), coll.to_string());
        let colls = self.collections.read().await;
        let indexes = self.indexes.read().await;

        let store = match colls.get(&key) {
            Some(s) => s,
            None => {
                return Ok(ResponsePayload::Records {
                    documents: Vec::new(),
                    count: 0,
                });
            }
        };

        let empty_idx_mgr = IndexManager::new();
        let idx_mgr = indexes.get(&key).unwrap_or(&empty_idx_mgr);

        // Plan query
        let plan = QueryPlanner::plan(filter.clone(), sort.clone(), limit, offset, idx_mgr);

        let candidate_ids: Vec<DocumentId> = match plan {
            QueryPlan::IndexLookup {
                ref index_name,
                ref key,
                ..
            } => {
                if let Some(idx) = idx_mgr.get(index_name) {
                    idx.lookup(key)
                } else {
                    store.keys().cloned().collect()
                }
            }
            QueryPlan::IndexRangeScan {
                ref index_name,
                ref range,
                ..
            } => {
                if let Some(idx) = idx_mgr.get(index_name) {
                    idx.scan(range).unwrap_or_default()
                } else {
                    store.keys().cloned().collect()
                }
            }
            QueryPlan::SeqScan { .. } => store.keys().cloned().collect(),
        };

        // Filter candidates
        let mut results: Vec<Document> = Vec::new();
        for id in candidate_ids {
            if let Some(doc) = store.get(&id) {
                if Evaluator::matches_filter(&filter, doc)? {
                    results.push(doc.clone());
                }
            }
        }

        // Sort results if requested
        if let Some(ref sort_clause) = sort {
            let field = &sort_clause.field;
            let dir = sort_clause.direction;
            results.sort_by(|a, b| {
                let v_a = a
                    .get_path(field)
                    .or_else(|| a.fields.get(field))
                    .unwrap_or(&Value::Null);
                let v_b = b
                    .get_path(field)
                    .or_else(|| b.fields.get(field))
                    .unwrap_or(&Value::Null);
                let ord = v_a.cmp(v_b);
                if dir == SortDirection::Desc {
                    ord.reverse()
                } else {
                    ord
                }
            });
        }

        // Apply offset & limit
        let off = offset.unwrap_or(0);
        let docs = if off >= results.len() {
            Vec::new()
        } else {
            let end = limit
                .map(|l| (off + l).min(results.len()))
                .unwrap_or(results.len());
            results[off..end].to_vec()
        };

        let count = docs.len();
        Ok(ResponsePayload::Records {
            documents: docs,
            count,
        })
    }

    async fn execute_update(
        &self,
        ns: &str,
        coll: &str,
        assignments: Vec<Assignment>,
        filter: Option<Expr>,
    ) -> Result<ResponsePayload> {
        let key = (ns.to_string(), coll.to_string());
        let mut updated_docs = Vec::new();

        {
            let mut colls = self.collections.write().await;
            let mut indexes = self.indexes.write().await;
            let store = colls.entry(key.clone()).or_default();
            let idx_mgr = indexes.entry(key.clone()).or_default();

            for (id, doc) in store.iter_mut() {
                if Evaluator::matches_filter(&filter, doc)? {
                    let before = doc.clone();
                    Evaluator::apply_assignments(doc, &assignments)?;
                    let after = doc.clone();

                    let payload = NbfRecord::payload_from_document(&after)?;
                    let seq =
                        self.wal
                            .append(RecordType::Update, ns, coll, id.clone(), payload, None)?;

                    let _ = idx_mgr.on_update(&before, &after);
                    updated_docs.push((seq, id.clone(), before, after));
                }
            }
        }

        let affected = updated_docs.len();

        // Emit events
        for (seq, id, before, after) in updated_docs {
            self.emit_event(DataEvent::new(
                seq,
                EventType::Update,
                ns,
                coll,
                id,
                Some(before),
                Some(after),
                None,
            ));
        }

        Ok(ResponsePayload::Success {
            message: format!("Updated {affected} document(s)"),
            affected,
        })
    }

    async fn execute_remove(
        &self,
        ns: &str,
        coll: &str,
        filter: Option<Expr>,
    ) -> Result<ResponsePayload> {
        let key = (ns.to_string(), coll.to_string());
        let mut removed_ids = Vec::new();

        {
            let mut colls = self.collections.write().await;
            let mut indexes = self.indexes.write().await;
            let store = colls.entry(key.clone()).or_default();
            let idx_mgr = indexes.entry(key.clone()).or_default();

            let target_ids: Vec<DocumentId> = store
                .iter()
                .filter(|(_, doc)| Evaluator::matches_filter(&filter, doc).unwrap_or(false))
                .map(|(id, _)| id.clone())
                .collect();

            for id in target_ids {
                if let Some(removed_doc) = store.remove(&id) {
                    let seq = self.wal.append(
                        RecordType::Delete,
                        ns,
                        coll,
                        id.clone(),
                        Vec::new(),
                        None,
                    )?;
                    let _ = idx_mgr.on_remove(&removed_doc);
                    removed_ids.push((seq, id, removed_doc));
                }
            }
        }

        let affected = removed_ids.len();

        for (seq, id, doc) in removed_ids {
            self.emit_event(DataEvent::new(
                seq,
                EventType::Delete,
                ns,
                coll,
                id,
                Some(doc),
                None,
                None,
            ));
        }

        Ok(ResponsePayload::Success {
            message: format!("Removed {affected} document(s)"),
            affected,
        })
    }

    async fn execute_count(
        &self,
        ns: &str,
        coll: &str,
        filter: Option<Expr>,
    ) -> Result<ResponsePayload> {
        let key = (ns.to_string(), coll.to_string());
        let colls = self.collections.read().await;

        let count = match colls.get(&key) {
            Some(store) => {
                let mut c = 0;
                for doc in store.values() {
                    if Evaluator::matches_filter(&filter, doc)? {
                        c += 1;
                    }
                }
                c
            }
            None => 0,
        };

        Ok(ResponsePayload::Count(count))
    }

    async fn execute_exists(
        &self,
        ns: &str,
        coll: &str,
        filter: Option<Expr>,
    ) -> Result<ResponsePayload> {
        let key = (ns.to_string(), coll.to_string());
        let colls = self.collections.read().await;

        let exists = match colls.get(&key) {
            Some(store) => {
                let mut found = false;
                for doc in store.values() {
                    if Evaluator::matches_filter(&filter, doc)? {
                        found = true;
                        break;
                    }
                }
                found
            }
            None => false,
        };

        Ok(ResponsePayload::Exists(exists))
    }

    async fn execute_create_index(
        &self,
        ns: &str,
        coll: &str,
        field: &str,
        index_type: IndexType,
    ) -> Result<ResponsePayload> {
        let key = (ns.to_string(), coll.to_string());
        let idx_name = format!("idx_{coll}_{field}");

        let colls = self.collections.read().await;
        let mut indexes = self.indexes.write().await;
        let idx_mgr = indexes.entry(key.clone()).or_default();

        let mut index: Box<dyn nova_index::Index> = match index_type {
            IndexType::Hash => Box::new(HashIndex::new(&idx_name, field)),
            IndexType::Ordered | IndexType::PrimaryKey => {
                Box::new(OrderedIndex::new(&idx_name, field))
            }
        };

        // Populate index from existing documents
        if let Some(store) = colls.get(&key) {
            for doc in store.values() {
                if let Some(val) = doc.get_path(field).or_else(|| doc.fields.get(field)) {
                    let _ = index.insert(val, &doc.id);
                }
            }
        }

        idx_mgr.add_index(index);
        info!(
            namespace = ns,
            collection = coll,
            field = field,
            "Index created"
        );

        Ok(ResponsePayload::Success {
            message: format!("Index '{idx_name}' created on '{coll}.{field}'"),
            affected: 1,
        })
    }

    async fn execute_drop_index(
        &self,
        ns: &str,
        coll: &str,
        field: &str,
    ) -> Result<ResponsePayload> {
        let key = (ns.to_string(), coll.to_string());
        let idx_name = format!("idx_{coll}_{field}");

        let mut indexes = self.indexes.write().await;
        if let Some(idx_mgr) = indexes.get_mut(&key) {
            let dropped = idx_mgr.drop_index(&idx_name);
            if dropped {
                return Ok(ResponsePayload::Success {
                    message: format!("Index '{idx_name}' dropped"),
                    affected: 1,
                });
            }
        }

        Err(NovaError::new(
            ErrorCode::IndexNotFound,
            format!("Index for '{field}' not found on '{coll}'"),
        ))
    }

    /// Read-only snapshot of current collections for snapshot creation.
    pub async fn snapshot_collections(&self) -> HashMap<CollectionKey, DocumentStore> {
        self.collections.read().await.clone()
    }

    /// Return structured system status for Web Studio and observability dashboards.
    pub async fn get_system_status(&self) -> serde_json::Value {
        let colls = self.collections.read().await;
        let indexes = self.indexes.read().await;
        let metrics = self.metrics.snapshot();

        let mut ns_map: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
        for ((ns, coll), store) in colls.iter() {
            let idx_count = indexes
                .get(&(ns.clone(), coll.clone()))
                .map(|m| m.list_indexes().len())
                .unwrap_or(0);
            let coll_info = serde_json::json!({
                "name": coll,
                "document_count": store.len(),
                "index_count": idx_count,
            });
            ns_map.entry(ns.clone()).or_default().push(coll_info);
        }

        let namespaces: Vec<serde_json::Value> = ns_map
            .into_iter()
            .map(|(name, collections)| {
                serde_json::json!({
                    "name": name,
                    "collections": collections,
                })
            })
            .collect();

        serde_json::json!({
            "status": "online",
            "version": "0.1.0",
            "tagline": "A database that understands the flow of your data.",
            "bind_addr": self.config.bind_addr(),
            "web_addr": self.config.web_addr(),
            "data_dir": self.config.data_dir.to_string_lossy(),
            "sync_mode": self.config.sync_mode,
            "max_connections": self.config.max_connections,
            "metrics": {
                "total_queries": metrics.total_queries,
                "total_writes": metrics.total_writes,
                "total_reads": metrics.total_reads,
                "total_events": metrics.total_events_emitted,
                "active_connections": metrics.active_connections,
            },
            "namespaces": namespaces,
        })
    }
}
