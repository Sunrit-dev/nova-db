use crate::format::NBF_VERSION;
use crate::recovery::CollectionKey;
use chrono::Utc;
use crc32fast::Hasher;
use nova_core::document::{Document, DocumentId};
use nova_core::error::{ErrorCode, NovaError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

/// Snapshot metadata describing a point-in-time database snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    pub id: String,
    pub timestamp: i64,
    pub database_version: String,
    pub storage_version: u16,
    pub document_count: usize,
    pub size_bytes: u64,
    pub checksum: u32,
}

/// Snapshot serialization payload containing document collections.
#[derive(Debug, Serialize, Deserialize)]
struct SnapshotData {
    pub collections: HashMap<String, HashMap<String, Document>>,
}

/// Snapshot management engine for creating, listing, and restoring point-in-time snapshots.
pub struct SnapshotManager;

impl SnapshotManager {
    /// Create a new snapshot from current in-memory collection state.
    pub fn create(
        snapshot_dir: &Path,
        snapshot_id: &str,
        collections: &HashMap<CollectionKey, HashMap<DocumentId, Document>>,
    ) -> Result<SnapshotMetadata> {
        fs::create_dir_all(snapshot_dir)
            .map_err(|e| NovaError::storage(format!("Failed to create snapshot directory: {e}")))?;

        let mut doc_count = 0;
        let mut flat_collections = HashMap::new();

        for ((ns, coll), doc_map) in collections {
            let key = format!("{ns}::{coll}");
            let mut string_doc_map = HashMap::new();
            for (id, doc) in doc_map {
                string_doc_map.insert(id.as_str().to_string(), doc.clone());
                doc_count += 1;
            }
            flat_collections.insert(key, string_doc_map);
        }

        let payload = SnapshotData {
            collections: flat_collections,
        };

        let raw_bytes = serde_json::to_vec(&payload)
            .map_err(|e| NovaError::storage(format!("Failed to serialize snapshot: {e}")))?;

        let mut hasher = Hasher::new();
        hasher.update(&raw_bytes);
        let checksum = hasher.finalize();

        let meta = SnapshotMetadata {
            id: snapshot_id.to_string(),
            timestamp: Utc::now().timestamp_micros(),
            database_version: env!("CARGO_PKG_VERSION").to_string(),
            storage_version: NBF_VERSION,
            document_count: doc_count,
            size_bytes: raw_bytes.len() as u64,
            checksum,
        };

        // Write snapshot data file: <snapshot_id>.snap
        let data_path = snapshot_dir.join(format!("{snapshot_id}.snap"));
        let mut data_file = File::create(&data_path).map_err(|e| {
            NovaError::storage(format!(
                "Failed to create snapshot file {:?}: {e}",
                data_path
            ))
        })?;
        data_file
            .write_all(&raw_bytes)
            .map_err(|e| NovaError::storage(e.to_string()))?;
        data_file
            .sync_all()
            .map_err(|e| NovaError::storage(e.to_string()))?;

        // Write snapshot metadata file: <snapshot_id>.meta.json
        let meta_path = snapshot_dir.join(format!("{snapshot_id}.meta.json"));
        let meta_json =
            serde_json::to_vec_pretty(&meta).map_err(|e| NovaError::storage(e.to_string()))?;
        let mut meta_file = File::create(&meta_path).map_err(|e| {
            NovaError::storage(format!("Failed to create meta file {:?}: {e}", meta_path))
        })?;
        meta_file
            .write_all(&meta_json)
            .map_err(|e| NovaError::storage(e.to_string()))?;
        meta_file
            .sync_all()
            .map_err(|e| NovaError::storage(e.to_string()))?;

        Ok(meta)
    }

    /// List all snapshots in the directory.
    pub fn list(snapshot_dir: &Path) -> Result<Vec<SnapshotMetadata>> {
        if !snapshot_dir.exists() {
            return Ok(Vec::new());
        }

        let mut snapshots = Vec::new();
        let entries = fs::read_dir(snapshot_dir)
            .map_err(|e| NovaError::storage(format!("Failed to read snapshot dir: {e}")))?;

        for entry in entries {
            let entry = entry.map_err(|e| NovaError::storage(e.to_string()))?;
            let path = entry.path();
            if path
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.ends_with(".meta.json"))
                .unwrap_or(false)
            {
                let meta_bytes = fs::read(&path).map_err(|e| NovaError::storage(e.to_string()))?;
                if let Ok(meta) = serde_json::from_slice::<SnapshotMetadata>(&meta_bytes) {
                    snapshots.push(meta);
                }
            }
        }

        snapshots.sort_by_key(|b| std::cmp::Reverse(b.timestamp));
        Ok(snapshots)
    }

    /// Restore documents and collections from a specified snapshot ID.
    #[allow(clippy::type_complexity)]
    pub fn restore(
        snapshot_dir: &Path,
        snapshot_id: &str,
    ) -> Result<(
        SnapshotMetadata,
        HashMap<CollectionKey, HashMap<DocumentId, Document>>,
    )> {
        let meta_path = snapshot_dir.join(format!("{snapshot_id}.meta.json"));
        let data_path = snapshot_dir.join(format!("{snapshot_id}.snap"));

        if !meta_path.exists() || !data_path.exists() {
            return Err(NovaError::new(
                ErrorCode::SnapshotFailed,
                format!("Snapshot '{snapshot_id}' does not exist"),
            ));
        }

        let meta_bytes = fs::read(&meta_path).map_err(|e| NovaError::storage(e.to_string()))?;
        let meta: SnapshotMetadata = serde_json::from_slice(&meta_bytes)
            .map_err(|e| NovaError::storage(format!("Corrupt snapshot metadata: {e}")))?;

        let mut data_file =
            File::open(&data_path).map_err(|e| NovaError::storage(e.to_string()))?;
        let mut raw_bytes = Vec::new();
        data_file
            .read_to_end(&mut raw_bytes)
            .map_err(|e| NovaError::storage(e.to_string()))?;

        // Verify checksum
        let mut hasher = Hasher::new();
        hasher.update(&raw_bytes);
        let calculated = hasher.finalize();

        if calculated != meta.checksum {
            return Err(NovaError::checksum_mismatch(meta.checksum, calculated));
        }

        let payload: SnapshotData = serde_json::from_slice(&raw_bytes)
            .map_err(|e| NovaError::storage(format!("Failed to deserialize snapshot: {e}")))?;

        let mut collections = HashMap::new();
        for (key_str, doc_map) in payload.collections {
            let parts: Vec<&str> = key_str.split("::").collect();
            if parts.len() != 2 {
                continue;
            }
            let key = (parts[0].to_string(), parts[1].to_string());
            let mut collection_map = HashMap::new();
            for (id_str, doc) in doc_map {
                if let Ok(doc_id) = DocumentId::new(id_str) {
                    collection_map.insert(doc_id, doc);
                }
            }
            collections.insert(key, collection_map);
        }

        Ok((meta, collections))
    }
}
