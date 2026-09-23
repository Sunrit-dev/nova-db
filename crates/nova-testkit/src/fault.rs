use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Fault injector for testing crash recovery and corruption resilience.
pub struct FaultInjector;

impl FaultInjector {
    /// Truncate file by `bytes` from the end to simulate power failure during a write.
    pub fn truncate_tail(path: &Path, bytes: u64) -> std::io::Result<()> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let len = file.metadata()?.len();
        let new_len = len.saturating_sub(bytes);
        file.set_len(new_len)?;
        file.sync_all()?;
        Ok(())
    }

    /// Corrupt bytes at a specific offset by flipping bits.
    pub fn flip_byte_at(path: &Path, offset: u64) -> std::io::Result<()> {
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut b = [0u8; 1];
        file.read_exact(&mut b)?;
        b[0] ^= 0xFF; // Flip all bits
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(&b)?;
        file.sync_all()?;
        Ok(())
    }

    /// Append arbitrary junk bytes to the tail of a file.
    pub fn append_garbage(path: &Path, garbage: &[u8]) -> std::io::Result<()> {
        let mut file = OpenOptions::new().append(true).open(path)?;
        file.write_all(garbage)?;
        file.sync_all()?;
        Ok(())
    }
}
