//! Storage engine for NOVA DB.

pub mod format;
pub mod recovery;
pub mod snapshot;
pub mod wal;

pub use format::{NbfRecord, RecordType, NBF_MAGIC, NBF_VERSION};
pub use recovery::{CollectionKey, RecoveredState, RecoveryManager};
pub use snapshot::{SnapshotManager, SnapshotMetadata};
pub use wal::{SyncMode, WriteAheadLog};

#[cfg(test)]
mod tests {
    use super::*;
    use nova_core::document::{Document, DocumentId};
    use nova_core::value::Value;
    use std::fs::OpenOptions;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_nbf_record_roundtrip() {
        let mut doc = Document::with_id("user_42");
        doc.insert("name", "Alice");
        doc.insert("score", 98.5);

        let payload = NbfRecord::payload_from_document(&doc).unwrap();
        let record = NbfRecord {
            sequence: 101,
            record_type: RecordType::Insert,
            flags: 0,
            tx_id: None,
            timestamp: 1700000000,
            namespace: "default".to_string(),
            collection: "users".to_string(),
            document_id: doc.id.clone(),
            payload,
        };

        let encoded = record.encode().unwrap();
        let mut cursor = std::io::Cursor::new(encoded);
        let decoded = NbfRecord::decode(&mut cursor)
            .unwrap()
            .expect("record should decode");

        assert_eq!(decoded.sequence, 101);
        assert_eq!(decoded.record_type, RecordType::Insert);
        assert_eq!(decoded.namespace, "default");
        assert_eq!(decoded.collection, "users");
        assert_eq!(decoded.document_id, doc.id);

        let restored_doc = decoded.document_from_payload().unwrap();
        assert_eq!(
            restored_doc.get("name"),
            Some(&Value::String("Alice".to_string()))
        );
    }

    #[test]
    fn test_nbf_zero_length_payload_record() {
        let record = NbfRecord {
            sequence: 404,
            record_type: RecordType::Delete,
            flags: 0,
            tx_id: Some(12345),
            timestamp: 1710000000,
            namespace: "prod".to_string(),
            collection: "tombstones".to_string(),
            document_id: DocumentId::new("doc_dead").unwrap(),
            payload: Vec::new(),
        };

        let encoded = record.encode().unwrap();
        let mut cursor = std::io::Cursor::new(encoded);
        let decoded = NbfRecord::decode(&mut cursor)
            .unwrap()
            .expect("zero-length payload record should decode");

        assert_eq!(decoded.sequence, 404);
        assert_eq!(decoded.record_type, RecordType::Delete);
        assert_eq!(decoded.tx_id, Some(12345));
        assert!(decoded.payload.is_empty());
    }

    #[test]
    fn test_wal_write_and_recovery() {
        let tmp = tempdir().unwrap();
        let wal_dir = tmp.path().join("wal");

        {
            let wal = WriteAheadLog::open(&wal_dir, SyncMode::Always, 1024 * 1024).unwrap();
            let mut doc1 = Document::with_id("u1");
            doc1.insert("name", "Bob");
            let p1 = NbfRecord::payload_from_document(&doc1).unwrap();
            wal.append(
                RecordType::Insert,
                "default",
                "users",
                doc1.id.clone(),
                p1,
                None,
            )
            .unwrap();

            let mut doc2 = Document::with_id("u2");
            doc2.insert("name", "Charlie");
            let p2 = NbfRecord::payload_from_document(&doc2).unwrap();
            wal.append(
                RecordType::Insert,
                "default",
                "users",
                doc2.id.clone(),
                p2,
                None,
            )
            .unwrap();

            wal.flush().unwrap();
        }

        // Recover from WAL
        let state = RecoveryManager::recover(&wal_dir).unwrap();
        assert_eq!(state.records_replayed, 2);
        assert_eq!(state.max_sequence, 2);

        let key = ("default".to_string(), "users".to_string());
        let users = state
            .collections
            .get(&key)
            .expect("users collection should exist");
        assert_eq!(users.len(), 2);
        assert_eq!(
            users.get(&"u1".into()).unwrap().get("name"),
            Some(&Value::String("Bob".to_string()))
        );
        assert_eq!(
            users.get(&"u2".into()).unwrap().get("name"),
            Some(&Value::String("Charlie".to_string()))
        );
    }

    #[test]
    fn test_recovery_with_torn_tail() {
        let tmp = tempdir().unwrap();
        let wal_dir = tmp.path().join("wal");

        {
            let wal = WriteAheadLog::open(&wal_dir, SyncMode::Always, 1024 * 1024).unwrap();
            let mut doc1 = Document::with_id("doc_valid");
            doc1.insert("status", "ok");
            let p1 = NbfRecord::payload_from_document(&doc1).unwrap();
            wal.append(
                RecordType::Insert,
                "default",
                "items",
                doc1.id.clone(),
                p1,
                None,
            )
            .unwrap();
            wal.flush().unwrap();
        }

        // Corrupt the tail by appending garbage bytes to the segment file
        let segment_path = WriteAheadLog::segment_path(&wal_dir, 0);
        {
            let mut file = OpenOptions::new().append(true).open(&segment_path).unwrap();
            // Incomplete header bytes simulating sudden power loss during write
            file.write_all(&[0x4E, 0x4F, 0x56, 0x42, 0x00, 0x01, 0x01])
                .unwrap();
            file.flush().unwrap();
        }

        // Recovery should safely detect the torn tail, truncate the corrupt trailing bytes, and recover valid record
        let state = RecoveryManager::recover(&wal_dir).unwrap();
        assert_eq!(state.records_replayed, 1);
        assert_eq!(state.torn_tails_truncated, 1);

        let key = ("default".to_string(), "items".to_string());
        let items = state.collections.get(&key).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items.get(&"doc_valid".into()).unwrap().get("status"),
            Some(&Value::String("ok".to_string()))
        );
    }

    #[test]
    fn test_snapshot_roundtrip() {
        let tmp = tempdir().unwrap();
        let snap_dir = tmp.path().join("snapshots");

        let mut collections = std::collections::HashMap::new();
        let mut user_map = std::collections::HashMap::new();
        let mut doc = Document::with_id("u100");
        doc.insert("email", "test@nova.dev");
        user_map.insert(doc.id.clone(), doc);
        collections.insert(("default".to_string(), "users".to_string()), user_map);

        let meta = SnapshotManager::create(&snap_dir, "snap_01", &collections).unwrap();
        assert_eq!(meta.id, "snap_01");
        assert_eq!(meta.document_count, 1);

        let list = SnapshotManager::list(&snap_dir).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "snap_01");

        let (restored_meta, restored_colls) =
            SnapshotManager::restore(&snap_dir, "snap_01").unwrap();
        assert_eq!(restored_meta.id, "snap_01");
        let users = restored_colls
            .get(&("default".to_string(), "users".to_string()))
            .unwrap();
        assert_eq!(
            users.get(&"u100".into()).unwrap().get("email"),
            Some(&Value::String("test@nova.dev".to_string()))
        );
    }
}
