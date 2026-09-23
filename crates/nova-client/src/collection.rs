use crate::client::NovaClient;
use crate::stream::EventStream;
use nova_core::document::Document;
use nova_core::error::{NovaError, Result};
use nova_protocol::ResponsePayload;

/// High-level collection handle for performing typed queries and mutations.
#[derive(Clone)]
pub struct CollectionHandle {
    client: NovaClient,
    collection: String,
}

impl CollectionHandle {
    pub fn new(client: NovaClient, collection: String) -> Self {
        Self { client, collection }
    }

    /// Insert a document into this collection.
    pub async fn insert(&self, doc: Document) -> Result<Document> {
        let json_str = serde_json::to_string(&doc.fields)
            .map_err(|e| NovaError::invalid_query(e.to_string()))?;
        let id_part = format!(r#""_id": "{}""#, doc.id);
        let fields_inner = if json_str == "{}" {
            format!("{{{id_part}}}")
        } else {
            let mut s = json_str.trim().to_string();
            s.pop(); // remove trailing '}'
            format!("{s}, {id_part}}}")
        };

        let nql = format!("INSERT INTO {} VALUES ({fields_inner})", self.collection);
        let resp = self.client.execute_nql(nql).await?;

        match resp {
            ResponsePayload::Records { mut documents, .. } if !documents.is_empty() => {
                Ok(documents.remove(0))
            }
            ResponsePayload::Success { .. } => Ok(doc),
            other => Err(NovaError::protocol(format!(
                "Unexpected insert response: {:?}",
                other
            ))),
        }
    }

    /// Query documents matching an NQL WHERE expression.
    pub async fn find(&self, filter: &str) -> Result<Vec<Document>> {
        let nql = format!("FIND {} WHERE {}", self.collection, filter);
        let resp = self.client.execute_nql(nql).await?;
        match resp {
            ResponsePayload::Records { documents, .. } => Ok(documents),
            other => Err(NovaError::protocol(format!(
                "Unexpected find response: {:?}",
                other
            ))),
        }
    }

    /// Query all documents in the collection.
    pub async fn find_all(&self) -> Result<Vec<Document>> {
        let nql = format!("FIND {}", self.collection);
        let resp = self.client.execute_nql(nql).await?;
        match resp {
            ResponsePayload::Records { documents, .. } => Ok(documents),
            other => Err(NovaError::protocol(format!(
                "Unexpected find_all response: {:?}",
                other
            ))),
        }
    }

    /// Find a single document by its exact primary key ID.
    pub async fn find_by_id(&self, id: &str) -> Result<Option<Document>> {
        let nql = format!("FIND {} WHERE id == \"{}\"", self.collection, id);
        let resp = self.client.execute_nql(nql).await?;
        match resp {
            ResponsePayload::Records { mut documents, .. } => Ok(documents.pop()),
            other => Err(NovaError::protocol(format!(
                "Unexpected find_by_id response: {:?}",
                other
            ))),
        }
    }

    /// Update documents matching filter with SET assignments.
    pub async fn update(&self, filter: &str, set_clause: &str) -> Result<usize> {
        let nql = format!(
            "UPDATE {} SET {} WHERE {}",
            self.collection, set_clause, filter
        );
        let resp = self.client.execute_nql(nql).await?;
        match resp {
            ResponsePayload::Success { affected, .. } => Ok(affected),
            other => Err(NovaError::protocol(format!(
                "Unexpected update response: {:?}",
                other
            ))),
        }
    }

    /// Remove documents matching filter expression.
    pub async fn remove(&self, filter: &str) -> Result<usize> {
        let nql = format!("REMOVE {} WHERE {}", self.collection, filter);
        let resp = self.client.execute_nql(nql).await?;
        match resp {
            ResponsePayload::Success { affected, .. } => Ok(affected),
            other => Err(NovaError::protocol(format!(
                "Unexpected remove response: {:?}",
                other
            ))),
        }
    }

    /// Count matching documents.
    pub async fn count(&self, filter: Option<&str>) -> Result<usize> {
        let nql = if let Some(f) = filter {
            format!("COUNT {} WHERE {}", self.collection, f)
        } else {
            format!("COUNT {}", self.collection)
        };
        let resp = self.client.execute_nql(nql).await?;
        match resp {
            ResponsePayload::Count(c) => Ok(c),
            other => Err(NovaError::protocol(format!(
                "Unexpected count response: {:?}",
                other
            ))),
        }
    }

    /// Check if at least one matching document exists.
    pub async fn exists(&self, filter: &str) -> Result<bool> {
        let nql = format!("EXISTS {} WHERE {}", self.collection, filter);
        let resp = self.client.execute_nql(nql).await?;
        match resp {
            ResponsePayload::Exists(b) => Ok(b),
            other => Err(NovaError::protocol(format!(
                "Unexpected exists response: {:?}",
                other
            ))),
        }
    }

    /// Create an index on a document field.
    pub async fn create_index(&self, field: &str, index_type: &str) -> Result<()> {
        let nql = format!(
            "CREATE INDEX {}.{} TYPE {}",
            self.collection, field, index_type
        );
        let resp = self.client.execute_nql(nql).await?;
        match resp {
            ResponsePayload::Success { .. } => Ok(()),
            other => Err(NovaError::protocol(format!(
                "Unexpected create_index response: {:?}",
                other
            ))),
        }
    }

    /// Drop an existing index.
    pub async fn drop_index(&self, field: &str) -> Result<()> {
        let nql = format!("DROP INDEX {}.{}", self.collection, field);
        let resp = self.client.execute_nql(nql).await?;
        match resp {
            ResponsePayload::Success { .. } => Ok(()),
            other => Err(NovaError::protocol(format!(
                "Unexpected drop_index response: {:?}",
                other
            ))),
        }
    }

    /// Subscribe to real-time change stream on this collection.
    pub async fn watch(&self, filter: Option<&str>) -> Result<EventStream> {
        let nql = if let Some(f) = filter {
            format!("WATCH {} WHERE {}", self.collection, f)
        } else {
            format!("WATCH {}", self.collection)
        };
        self.client.register_watch(nql).await
    }
}
