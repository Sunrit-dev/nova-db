use crate::format::{NbfRecord, RecordType};
use crate::wal::WriteAheadLog;
use nova_core::document::{Document, DocumentId};
use nova_core::error::{ErrorCode, NovaError, Result};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{BufReader, Seek};
use std::path::Path;
use tracing::{info, warn};

/// Collection key: (namespace, collection)
pub type CollectionKey = (String, String);

/// Recovered database state from WAL replay.
#[derive(Debug)]
pub struct RecoveredState {
    /// Documents grouped by (namespace, collection) -> DocumentId -> Document
    pub collections: HashMap<CollectionKey, HashMap<DocumentId, Document>>,
    /// Highest sequence number found in valid records
    pub max_sequence: u64,
    /// Total number of valid records replayed
    pub records_replayed: usize,
    /// Number of torn tails truncated
    pub torn_tails_truncated: usize,
}

impl RecoveredState {
    pub fn new() -> Self {
        Self {
            collections: HashMap::new(),
            max_sequence: 0,
            records_replayed: 0,
            torn_tails_truncated: 0,
        }
    }
}

impl Default for RecoveredState {
    fn default() -> Self {
        Self::new()
    }
}

/// Recovery engine for discovering, validating, repairing, and replaying WAL logs.
pub struct RecoveryManager;

impl RecoveryManager {
    /// Run full crash recovery over the specified WAL directory.
    pub fn recover(wal_dir: &Path) -> Result<RecoveredState> {
        info!(dir = ?wal_dir, "Beginning crash recovery scan");

        if !wal_dir.exists() {
            info!("WAL directory does not exist. Initializing clean state.");
            return Ok(RecoveredState::new());
        }

        let segments = WriteAheadLog::discover_segments(wal_dir)?;
        let mut state = RecoveredState::new();

        if segments.is_empty() {
            info!("No existing WAL segments found. Initializing clean state.");
            return Ok(state);
        }

        for &segment_idx in &segments {
            let segment_path = WriteAheadLog::segment_path(wal_dir, segment_idx);
            Self::recover_segment(&segment_path, &mut state)?;
        }

        info!(
            records_replayed = state.records_replayed,
            max_sequence = state.max_sequence,
            torn_tails_truncated = state.torn_tails_truncated,
            "Crash recovery completed successfully"
        );

        Ok(state)
    }

    /// Scan a single segment file, validate CRC32, repair torn tail if found, and replay mutations.
    fn recover_segment(path: &Path, state: &mut RecoveredState) -> Result<()> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| {
                NovaError::storage(format!("Failed to open segment file {:?}: {e}", path))
            })?;

        let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        if file_len == 0 {
            return Ok(());
        }

        let mut reader = BufReader::new(&mut file);
        let mut last_valid_offset = 0u64;

        loop {
            let offset_before = match reader.stream_position() {
                Ok(pos) => pos,
                Err(e) => {
                    return Err(NovaError::storage(format!(
                        "Failed to get stream position: {e}"
                    )))
                }
            };

            match NbfRecord::decode(&mut reader) {
                Ok(Some(record)) => {
                    // Update state with valid record
                    if record.sequence > state.max_sequence {
                        state.max_sequence = record.sequence;
                    }
                    Self::apply_record_to_state(&record, state)?;
                    state.records_replayed += 1;
                    last_valid_offset = reader
                        .stream_position()
                        .map_err(|e| NovaError::storage(e.to_string()))?;
                }
                Ok(None) => {
                    // Clean EOF reached
                    break;
                }
                Err(err) if err.code == ErrorCode::TornWriteDetected => {
                    // Torn write at the tail detected (partial write before crash)
                    warn!(
                        path = ?path,
                        offset = offset_before,
                        last_valid_offset = last_valid_offset,
                        "Torn write detected at tail. Safely truncating segment."
                    );
                    drop(reader);
                    file.set_len(last_valid_offset).map_err(|e| {
                        NovaError::storage(format!(
                            "Failed to truncate torn tail in {:?}: {e}",
                            path
                        ))
                    })?;
                    file.sync_all()
                        .map_err(|e| NovaError::storage(e.to_string()))?;
                    state.torn_tails_truncated += 1;
                    break;
                }
                Err(err) => {
                    // Checksum mismatch or corruption in validated stream
                    return Err(NovaError::with_details(
                        ErrorCode::WalCorrupted,
                        format!(
                            "Corruption encountered in WAL segment {:?} at offset {offset_before}",
                            path
                        ),
                        err.to_string(),
                    ));
                }
            }
        }

        Ok(())
    }

    /// Apply a validated record to the in-memory document state during recovery replay.
    fn apply_record_to_state(record: &NbfRecord, state: &mut RecoveredState) -> Result<()> {
        let key = (record.namespace.clone(), record.collection.clone());
        let collection_map = state.collections.entry(key).or_default();

        match record.record_type {
            RecordType::Insert | RecordType::Update => {
                let doc = record.document_from_payload()?;
                collection_map.insert(record.document_id.clone(), doc);
            }
            RecordType::Delete => {
                collection_map.remove(&record.document_id);
            }
            RecordType::Checkpoint
            | RecordType::TxBegin
            | RecordType::TxCommit
            | RecordType::TxRollback => {
                // Informational or transaction lifecycle records
            }
        }

        Ok(())
    }
}
