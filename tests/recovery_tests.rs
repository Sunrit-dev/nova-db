use nova_core::document::Document;
use nova_core::value::Value;
use nova_storage::format::RecordType;
use nova_storage::recovery::RecoveryManager;
use nova_storage::wal::{SyncMode, WriteAheadLog};
use nova_testkit::FaultInjector;
use tempfile::tempdir;

#[test]
fn test_recovery_from_truncated_wal_tail() {
    let tmp = tempdir().unwrap();
    let wal_dir = tmp.path().join("wal");

    // 1. Write several valid records
    {
        let wal = WriteAheadLog::open(&wal_dir, SyncMode::Always, 1024 * 1024).unwrap();
        for i in 0..10 {
            let mut doc = Document::with_id(format!("item_{i}"));
            doc.insert("seq", i as i64);
            let payload = nova_storage::format::NbfRecord::payload_from_document(&doc).unwrap();
            wal.append(RecordType::Insert, "default", "inventory", doc.id, payload, None).unwrap();
        }
        wal.flush().unwrap();
    }

    // 2. Simulate partial write by truncating last 15 bytes of active segment
    let segment_path = WriteAheadLog::segment_path(&wal_dir, 0);
    FaultInjector::truncate_tail(&segment_path, 15).unwrap();

    // 3. Recovery should detect torn write, truncate cleanly to 9 valid records
    let state = RecoveryManager::recover(&wal_dir).unwrap();
    assert_eq!(state.records_replayed, 9);
    assert_eq!(state.torn_tails_truncated, 1);

    let key = ("default".to_string(), "inventory".to_string());
    let inv = state.collections.get(&key).unwrap();
    assert_eq!(inv.len(), 9);
}

#[test]
fn test_recovery_detects_corrupted_checksum() {
    let tmp = tempdir().unwrap();
    let wal_dir = tmp.path().join("wal");

    {
        let wal = WriteAheadLog::open(&wal_dir, SyncMode::Always, 1024 * 1024).unwrap();
        let mut doc = Document::with_id("critical_doc");
        doc.insert("balance", 1_000_000);
        let payload = nova_storage::format::NbfRecord::payload_from_document(&doc).unwrap();
        wal.append(RecordType::Insert, "default", "accounts", doc.id, payload, None).unwrap();
        wal.flush().unwrap();
    }

    // Corrupt a byte in the payload
    let segment_path = WriteAheadLog::segment_path(&wal_dir, 0);
    FaultInjector::flip_byte_at(&segment_path, 35).unwrap();

    // Recovery must reject corrupt data
    let res = RecoveryManager::recover(&wal_dir);
    assert!(res.is_err(), "Corrupt WAL record with invalid checksum must fail recovery");
}
