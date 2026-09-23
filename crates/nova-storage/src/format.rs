use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use crc32fast::Hasher;
use nova_core::document::{Document, DocumentId};
use nova_core::error::{ErrorCode, NovaError, Result};
use std::io::{Cursor, Read};

/// Magic bytes for NOVA Binary Format: "NOVB"
pub const NBF_MAGIC: [u8; 4] = [0x4E, 0x4F, 0x56, 0x42];
/// Current NBF storage version
pub const NBF_VERSION: u16 = 1;

/// Type of WAL / NBF record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RecordType {
    Insert = 0x01,
    Update = 0x02,
    Delete = 0x03,
    Checkpoint = 0x04,
    TxBegin = 0x05,
    TxCommit = 0x06,
    TxRollback = 0x07,
}

impl RecordType {
    pub fn from_u8(b: u8) -> Result<Self> {
        match b {
            0x01 => Ok(RecordType::Insert),
            0x02 => Ok(RecordType::Update),
            0x03 => Ok(RecordType::Delete),
            0x04 => Ok(RecordType::Checkpoint),
            0x05 => Ok(RecordType::TxBegin),
            0x06 => Ok(RecordType::TxCommit),
            0x07 => Ok(RecordType::TxRollback),
            _ => Err(NovaError::storage(format!(
                "Unknown NBF record type: 0x{b:02X}"
            ))),
        }
    }
}

/// A structured record in the NOVA Binary Format.
#[derive(Debug, Clone, PartialEq)]
pub struct NbfRecord {
    pub sequence: u64,
    pub record_type: RecordType,
    pub flags: u8,
    pub tx_id: Option<u64>,
    pub timestamp: u64,
    pub namespace: String,
    pub collection: String,
    pub document_id: DocumentId,
    pub payload: Vec<u8>,
}

impl NbfRecord {
    /// Serialize this record into bytes with CRC32 checksum.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(64 + self.payload.len());

        buf.extend_from_slice(&NBF_MAGIC);
        buf.write_u16::<BigEndian>(NBF_VERSION)
            .map_err(|e| NovaError::storage(e.to_string()))?;
        buf.write_u8(self.record_type as u8)
            .map_err(|e| NovaError::storage(e.to_string()))?;
        buf.write_u8(self.flags)
            .map_err(|e| NovaError::storage(e.to_string()))?;
        buf.write_u64::<BigEndian>(self.sequence)
            .map_err(|e| NovaError::storage(e.to_string()))?;
        buf.write_u64::<BigEndian>(self.tx_id.unwrap_or(0))
            .map_err(|e| NovaError::storage(e.to_string()))?;
        buf.write_u64::<BigEndian>(self.timestamp)
            .map_err(|e| NovaError::storage(e.to_string()))?;

        let ns_bytes = self.namespace.as_bytes();
        if ns_bytes.len() > u16::MAX as usize {
            return Err(NovaError::storage("Namespace length exceeds u16 limit"));
        }
        buf.write_u16::<BigEndian>(ns_bytes.len() as u16)
            .map_err(|e| NovaError::storage(e.to_string()))?;
        buf.extend_from_slice(ns_bytes);

        let coll_bytes = self.collection.as_bytes();
        if coll_bytes.len() > u16::MAX as usize {
            return Err(NovaError::storage(
                "Collection name length exceeds u16 limit",
            ));
        }
        buf.write_u16::<BigEndian>(coll_bytes.len() as u16)
            .map_err(|e| NovaError::storage(e.to_string()))?;
        buf.extend_from_slice(coll_bytes);

        let doc_id_bytes = self.document_id.as_str().as_bytes();
        if doc_id_bytes.len() > u16::MAX as usize {
            return Err(NovaError::storage("Document ID length exceeds u16 limit"));
        }
        buf.write_u16::<BigEndian>(doc_id_bytes.len() as u16)
            .map_err(|e| NovaError::storage(e.to_string()))?;
        buf.extend_from_slice(doc_id_bytes);

        if self.payload.len() > 64 * 1024 * 1024 {
            return Err(NovaError::storage("Payload exceeds 64MB limit"));
        }
        buf.write_u32::<BigEndian>(self.payload.len() as u32)
            .map_err(|e| NovaError::storage(e.to_string()))?;
        buf.extend_from_slice(&self.payload);

        // Compute CRC32 over the entire record before checksum
        let mut hasher = Hasher::new();
        hasher.update(&buf);
        let checksum = hasher.finalize();

        buf.write_u32::<BigEndian>(checksum)
            .map_err(|e| NovaError::storage(e.to_string()))?;

        Ok(buf)
    }

    /// Decode a record from a reader. Returns Ok(Some(record)) on success,
    /// Ok(None) on clean EOF, or an error (including ChecksumMismatch or TornWriteDetected).
    pub fn decode<R: Read>(reader: &mut R) -> Result<Option<Self>> {
        let mut magic = [0u8; 4];
        match reader.read_exact(&mut magic) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(NovaError::storage(format!("Failed reading magic: {e}"))),
        }

        if magic != NBF_MAGIC {
            return Err(NovaError::with_details(
                ErrorCode::WalCorrupted,
                "Invalid NBF magic bytes",
                format!("Expected {:?}, found {:?}", NBF_MAGIC, magic),
            ));
        }

        let mut header_buf = [0u8; 28]; // 2 (version) + 1 (type) + 1 (flags) + 8 (seq) + 8 (tx) + 8 (ts)
        if let Err(e) = reader.read_exact(&mut header_buf) {
            return Err(NovaError::with_details(
                ErrorCode::TornWriteDetected,
                "Incomplete NBF record header (torn write)",
                e.to_string(),
            ));
        }

        let mut r = Cursor::new(&header_buf);
        let version = r
            .read_u16::<BigEndian>()
            .map_err(|e| NovaError::storage(e.to_string()))?;
        if version != NBF_VERSION {
            return Err(NovaError::storage(format!(
                "Unsupported NBF version: {version}"
            )));
        }

        let record_type_byte = r.read_u8().map_err(|e| NovaError::storage(e.to_string()))?;
        let record_type = RecordType::from_u8(record_type_byte)?;
        let flags = r.read_u8().map_err(|e| NovaError::storage(e.to_string()))?;
        let sequence = r
            .read_u64::<BigEndian>()
            .map_err(|e| NovaError::storage(e.to_string()))?;
        let raw_tx = r
            .read_u64::<BigEndian>()
            .map_err(|e| NovaError::storage(e.to_string()))?;
        let tx_id = if raw_tx == 0 { None } else { Some(raw_tx) };
        let timestamp = r
            .read_u64::<BigEndian>()
            .map_err(|e| NovaError::storage(e.to_string()))?;

        // Read namespace
        let ns_len = match reader.read_u16::<BigEndian>() {
            Ok(len) => len as usize,
            Err(e) => {
                return Err(NovaError::new(
                    ErrorCode::TornWriteDetected,
                    format!("Torn write in namespace length: {e}"),
                ))
            }
        };
        let mut ns_bytes = vec![0u8; ns_len];
        if let Err(e) = reader.read_exact(&mut ns_bytes) {
            return Err(NovaError::new(
                ErrorCode::TornWriteDetected,
                format!("Torn write in namespace: {e}"),
            ));
        }
        let namespace = String::from_utf8(ns_bytes)
            .map_err(|_| NovaError::wal_corrupted("Invalid UTF-8 in namespace"))?;

        // Read collection
        let coll_len = match reader.read_u16::<BigEndian>() {
            Ok(len) => len as usize,
            Err(e) => {
                return Err(NovaError::new(
                    ErrorCode::TornWriteDetected,
                    format!("Torn write in collection length: {e}"),
                ))
            }
        };
        let mut coll_bytes = vec![0u8; coll_len];
        if let Err(e) = reader.read_exact(&mut coll_bytes) {
            return Err(NovaError::new(
                ErrorCode::TornWriteDetected,
                format!("Torn write in collection: {e}"),
            ));
        }
        let collection = String::from_utf8(coll_bytes)
            .map_err(|_| NovaError::wal_corrupted("Invalid UTF-8 in collection"))?;

        // Read document ID
        let doc_id_len = match reader.read_u16::<BigEndian>() {
            Ok(len) => len as usize,
            Err(e) => {
                return Err(NovaError::new(
                    ErrorCode::TornWriteDetected,
                    format!("Torn write in doc ID length: {e}"),
                ))
            }
        };
        let mut doc_id_bytes = vec![0u8; doc_id_len];
        if let Err(e) = reader.read_exact(&mut doc_id_bytes) {
            return Err(NovaError::new(
                ErrorCode::TornWriteDetected,
                format!("Torn write in doc ID: {e}"),
            ));
        }
        let doc_id_str = String::from_utf8(doc_id_bytes)
            .map_err(|_| NovaError::wal_corrupted("Invalid UTF-8 in document ID"))?;
        let document_id = DocumentId::new(doc_id_str)?;

        // Read payload
        let payload_len = match reader.read_u32::<BigEndian>() {
            Ok(len) => len as usize,
            Err(e) => {
                return Err(NovaError::new(
                    ErrorCode::TornWriteDetected,
                    format!("Torn write in payload length: {e}"),
                ))
            }
        };
        if payload_len > 64 * 1024 * 1024 {
            return Err(NovaError::wal_corrupted("Payload length exceeds limit"));
        }

        let mut payload = vec![0u8; payload_len];
        if let Err(e) = reader.read_exact(&mut payload) {
            return Err(NovaError::new(
                ErrorCode::TornWriteDetected,
                format!("Torn write in payload: {e}"),
            ));
        }

        // Read checksum
        let expected_checksum = match reader.read_u32::<BigEndian>() {
            Ok(cs) => cs,
            Err(e) => {
                return Err(NovaError::new(
                    ErrorCode::TornWriteDetected,
                    format!("Torn write in checksum: {e}"),
                ))
            }
        };

        // Recompute checksum over entire content
        let mut hasher = Hasher::new();
        hasher.update(&magic);
        hasher.update(&header_buf);
        hasher.update(&(ns_len as u16).to_be_bytes());
        hasher.update(namespace.as_bytes());
        hasher.update(&(coll_len as u16).to_be_bytes());
        hasher.update(collection.as_bytes());
        hasher.update(&(doc_id_len as u16).to_be_bytes());
        hasher.update(document_id.as_str().as_bytes());
        hasher.update(&(payload_len as u32).to_be_bytes());
        hasher.update(&payload);

        let calculated_checksum = hasher.finalize();
        if expected_checksum != calculated_checksum {
            return Err(NovaError::checksum_mismatch(
                expected_checksum,
                calculated_checksum,
            ));
        }

        Ok(Some(NbfRecord {
            sequence,
            record_type,
            flags,
            tx_id,
            timestamp,
            namespace,
            collection,
            document_id,
            payload,
        }))
    }

    /// Serialize a document into payload bytes.
    pub fn payload_from_document(doc: &Document) -> Result<Vec<u8>> {
        serde_json::to_vec(doc).map_err(|e| NovaError::storage(e.to_string()))
    }

    /// Deserialize a document from payload bytes.
    pub fn document_from_payload(&self) -> Result<Document> {
        serde_json::from_slice(&self.payload).map_err(|e| NovaError::storage(e.to_string()))
    }
}
