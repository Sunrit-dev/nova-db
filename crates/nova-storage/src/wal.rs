use crate::format::{NbfRecord, RecordType};
use chrono::Utc;
use nova_core::document::DocumentId;
use nova_core::error::{NovaError, Result};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tracing::info;

/// Durability sync policy for commit log flushes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    /// Fsync to physical storage on every mutation commit.
    Always,
    /// Rely on operating system page cache flushes.
    None,
}

/// Write-Ahead Log (WAL) manager with segment rotation and durability policies.
pub struct WriteAheadLog {
    dir: PathBuf,
    current_segment_index: AtomicU64,
    current_sequence: AtomicU64,
    segment_size_limit: u64,
    sync_mode: SyncMode,
    active_file: Mutex<BufWriter<File>>,
    current_segment_size: AtomicU64,
}

impl WriteAheadLog {
    /// Open or create a WAL directory with the specified configuration.
    pub fn open(
        dir: impl AsRef<Path>,
        sync_mode: SyncMode,
        segment_size_limit: u64,
    ) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)
            .map_err(|e| NovaError::storage(format!("Failed to create WAL dir: {e}")))?;

        // Discover existing segments
        let segments = Self::discover_segments(&dir)?;
        let (current_segment_index, current_sequence) = if let Some(last) = segments.last() {
            (*last, 0) // Sequence will be re-aligned during recovery scan
        } else {
            (0, 0)
        };

        let segment_path = Self::segment_path(&dir, current_segment_index);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&segment_path)
            .map_err(|e| {
                NovaError::storage(format!("Failed to open segment {:?}: {e}", segment_path))
            })?;

        let initial_size = file.metadata().map(|m| m.len()).unwrap_or(0);
        let writer = BufWriter::new(file);

        Ok(Self {
            dir,
            current_segment_index: AtomicU64::new(current_segment_index),
            current_sequence: AtomicU64::new(current_sequence),
            segment_size_limit,
            sync_mode,
            active_file: Mutex::new(writer),
            current_segment_size: AtomicU64::new(initial_size),
        })
    }

    /// Set sequence counter directly (typically called after recovery replay).
    pub fn set_sequence(&self, seq: u64) {
        self.current_sequence.store(seq, Ordering::SeqCst);
    }

    /// Returns the next monotonic sequence number.
    pub fn next_sequence(&self) -> u64 {
        self.current_sequence.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Returns the current sequence number.
    pub fn current_sequence(&self) -> u64 {
        self.current_sequence.load(Ordering::SeqCst)
    }

    /// Append a new mutation record to the WAL.
    pub fn append(
        &self,
        record_type: RecordType,
        namespace: impl Into<String>,
        collection: impl Into<String>,
        document_id: DocumentId,
        payload: Vec<u8>,
        tx_id: Option<u64>,
    ) -> Result<u64> {
        let sequence = self.next_sequence();
        let timestamp = Utc::now().timestamp_micros() as u64;

        let record = NbfRecord {
            sequence,
            record_type,
            flags: 0,
            tx_id,
            timestamp,
            namespace: namespace.into(),
            collection: collection.into(),
            document_id,
            payload,
        };

        let encoded = record.encode()?;
        let record_len = encoded.len() as u64;

        // Check if segment rotation is needed
        let current_size = self.current_segment_size.load(Ordering::SeqCst);
        if current_size + record_len > self.segment_size_limit {
            self.rotate_segment()?;
        }

        let mut writer = self
            .active_file
            .lock()
            .map_err(|_| NovaError::storage("Active WAL file lock poisoned"))?;

        // Check for fault injection: partial_write simulation
        if let Ok(fault) = std::env::var("NOVA_FAULT") {
            if fault == "partial_write" {
                // Write only half the bytes to simulate power loss during I/O
                let partial_len = encoded.len().saturating_sub(16);
                writer
                    .write_all(&encoded[..partial_len])
                    .map_err(|e| NovaError::storage(format!("WAL partial write: {e}")))?;
                writer
                    .flush()
                    .map_err(|e| NovaError::storage(e.to_string()))?;
                return Err(NovaError::storage("Simulated partial write fault injected"));
            }
        }

        writer
            .write_all(&encoded)
            .map_err(|e| NovaError::storage(format!("Failed to write WAL record: {e}")))?;

        if self.sync_mode == SyncMode::Always {
            writer
                .flush()
                .map_err(|e| NovaError::storage(format!("Failed to flush WAL buffer: {e}")))?;
            writer
                .get_ref()
                .sync_all()
                .map_err(|e| NovaError::storage(format!("Failed to fsync WAL file: {e}")))?;
        }

        self.current_segment_size
            .fetch_add(record_len, Ordering::SeqCst);

        // Check for fault injection: crash_after_wal simulation
        if let Ok(fault) = std::env::var("NOVA_FAULT") {
            if fault == "crash_after_wal" {
                panic!("NOVA_FAULT=crash_after_wal triggered: process simulating crash right after WAL append");
            }
        }

        Ok(sequence)
    }

    /// Flush and fsync the active log file to disk.
    pub fn flush(&self) -> Result<()> {
        let mut writer = self
            .active_file
            .lock()
            .map_err(|_| NovaError::storage("Active WAL file lock poisoned"))?;
        writer
            .flush()
            .map_err(|e| NovaError::storage(format!("Failed to flush WAL: {e}")))?;
        writer
            .get_ref()
            .sync_all()
            .map_err(|e| NovaError::storage(format!("Failed to sync WAL: {e}")))?;
        Ok(())
    }

    /// Rotate active segment to a new segment file.
    fn rotate_segment(&self) -> Result<()> {
        let mut writer = self
            .active_file
            .lock()
            .map_err(|_| NovaError::storage("Active WAL file lock poisoned"))?;
        writer
            .flush()
            .map_err(|e| NovaError::storage(e.to_string()))?;
        writer
            .get_ref()
            .sync_all()
            .map_err(|e| NovaError::storage(e.to_string()))?;

        let next_index = self.current_segment_index.fetch_add(1, Ordering::SeqCst) + 1;
        let new_path = Self::segment_path(&self.dir, next_index);

        info!(segment = next_index, path = ?new_path, "Rotating to new WAL segment");

        let new_file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&new_path)
            .map_err(|e| {
                NovaError::storage(format!("Failed to open new segment {:?}: {e}", new_path))
            })?;

        *writer = BufWriter::new(new_file);
        self.current_segment_size.store(0, Ordering::SeqCst);

        Ok(())
    }

    /// Discover existing segment indices sorted in ascending order.
    pub fn discover_segments(dir: &Path) -> Result<Vec<u64>> {
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut segments = Vec::new();
        let entries = fs::read_dir(dir)
            .map_err(|e| NovaError::storage(format!("Failed to read WAL directory: {e}")))?;

        for entry in entries {
            let entry = entry.map_err(|e| NovaError::storage(e.to_string()))?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("wal") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if let Ok(idx) = stem.parse::<u64>() {
                        segments.push(idx);
                    }
                }
            }
        }

        segments.sort_unstable();
        Ok(segments)
    }

    /// Compute file path for a segment index: `0000000000000001.wal`.
    pub fn segment_path(dir: &Path, index: u64) -> PathBuf {
        dir.join(format!("{:016}.wal", index))
    }
}
