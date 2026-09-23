use std::fmt;
use thiserror::Error;

/// Structured error codes for NOVA DB operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ErrorCode {
    DocumentNotFound,
    CollectionNotFound,
    NamespaceNotFound,
    DocumentAlreadyExists,
    IndexAlreadyExists,
    IndexNotFound,
    InvalidQuery,
    ParseError,
    TypeError,
    StorageError,
    WalCorrupted,
    TornWriteDetected,
    ChecksumMismatch,
    TransactionAborted,
    TransactionActive,
    NoActiveTransaction,
    ProtocolViolation,
    FrameTooLarge,
    AuthenticationFailed,
    PermissionDenied,
    ConnectionLimitExceeded,
    BufferOverflow,
    SnapshotFailed,
    InternalError,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::DocumentNotFound => "DOCUMENT_NOT_FOUND",
            ErrorCode::CollectionNotFound => "COLLECTION_NOT_FOUND",
            ErrorCode::NamespaceNotFound => "NAMESPACE_NOT_FOUND",
            ErrorCode::DocumentAlreadyExists => "DOCUMENT_ALREADY_EXISTS",
            ErrorCode::IndexAlreadyExists => "INDEX_ALREADY_EXISTS",
            ErrorCode::IndexNotFound => "INDEX_NOT_FOUND",
            ErrorCode::InvalidQuery => "INVALID_QUERY",
            ErrorCode::ParseError => "PARSE_ERROR",
            ErrorCode::TypeError => "TYPE_ERROR",
            ErrorCode::StorageError => "STORAGE_ERROR",
            ErrorCode::WalCorrupted => "WAL_CORRUPTED",
            ErrorCode::TornWriteDetected => "TORN_WRITE_DETECTED",
            ErrorCode::ChecksumMismatch => "CHECKSUM_MISMATCH",
            ErrorCode::TransactionAborted => "TRANSACTION_ABORTED",
            ErrorCode::TransactionActive => "TRANSACTION_ACTIVE",
            ErrorCode::NoActiveTransaction => "NO_ACTIVE_TRANSACTION",
            ErrorCode::ProtocolViolation => "PROTOCOL_VIOLATION",
            ErrorCode::FrameTooLarge => "FRAME_TOO_LARGE",
            ErrorCode::AuthenticationFailed => "AUTH_FAILED",
            ErrorCode::PermissionDenied => "PERMISSION_DENIED",
            ErrorCode::ConnectionLimitExceeded => "CONNECTION_LIMIT_EXCEEDED",
            ErrorCode::BufferOverflow => "BUFFER_OVERFLOW",
            ErrorCode::SnapshotFailed => "SNAPSHOT_FAILED",
            ErrorCode::InternalError => "INTERNAL_ERROR",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Unified error type for NOVA DB with actionable diagnostics.
#[derive(Error, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NovaError {
    pub code: ErrorCode,
    pub message: String,
    pub details: Option<String>,
}

impl NovaError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(
        code: ErrorCode,
        message: impl Into<String>,
        details: impl Into<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            details: Some(details.into()),
        }
    }

    pub fn document_not_found(id: impl Into<String>) -> Self {
        Self::new(
            ErrorCode::DocumentNotFound,
            format!("Document with ID '{}' was not found", id.into()),
        )
    }

    pub fn collection_not_found(name: impl Into<String>) -> Self {
        Self::new(
            ErrorCode::CollectionNotFound,
            format!("Collection '{}' does not exist", name.into()),
        )
    }

    pub fn invalid_query(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidQuery, msg)
    }

    pub fn parse_error(msg: impl Into<String>, line: usize, col: usize) -> Self {
        Self::with_details(
            ErrorCode::ParseError,
            msg,
            format!("Error location: line {line}, column {col}"),
        )
    }

    pub fn type_error(expected: &str, found: &str) -> Self {
        Self::new(
            ErrorCode::TypeError,
            format!("Type mismatch: expected {expected}, found {found}"),
        )
    }

    pub fn storage(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::StorageError, msg)
    }

    pub fn wal_corrupted(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::WalCorrupted, msg)
    }

    pub fn checksum_mismatch(expected: u32, calculated: u32) -> Self {
        Self::with_details(
            ErrorCode::ChecksumMismatch,
            "Checksum verification failed",
            format!("Expected CRC32 0x{expected:08X}, but calculated 0x{calculated:08X}"),
        )
    }

    pub fn protocol(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::ProtocolViolation, msg)
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::InternalError, msg)
    }
}

impl fmt::Display for NovaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)?;
        if let Some(ref details) = self.details {
            write!(f, " ({details})")?;
        }
        Ok(())
    }
}

pub type Result<T> = std::result::Result<T, NovaError>;
