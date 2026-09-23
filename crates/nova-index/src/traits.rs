use nova_core::document::DocumentId;
use nova_core::value::Value;
use std::ops::Bound;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexType {
    PrimaryKey,
    Hash,
    Ordered,
}

#[derive(Error, Debug, Clone, PartialEq)]
pub enum IndexError {
    #[error("Duplicate key violation on index '{0}'")]
    DuplicateKey(String),
    #[error("Index '{0}' does not support range scans")]
    ScanNotSupported(String),
    #[error("Key not found in index '{0}'")]
    KeyNotFound(String),
    #[error("Type mismatch for index on field '{field}': expected {expected}, found {found}")]
    TypeMismatch {
        field: String,
        expected: &'static str,
        found: &'static str,
    },
}

/// Range query definition for ordered indexes.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexRange {
    pub start: Bound<Value>,
    pub end: Bound<Value>,
}

impl IndexRange {
    pub fn all() -> Self {
        Self {
            start: Bound::Unbounded,
            end: Bound::Unbounded,
        }
    }

    pub fn exact(val: Value) -> Self {
        Self {
            start: Bound::Included(val.clone()),
            end: Bound::Included(val),
        }
    }

    pub fn from_inclusive(val: Value) -> Self {
        Self {
            start: Bound::Included(val),
            end: Bound::Unbounded,
        }
    }

    pub fn to_inclusive(val: Value) -> Self {
        Self {
            start: Bound::Unbounded,
            end: Bound::Included(val),
        }
    }

    pub fn between_inclusive(start: Value, end: Value) -> Self {
        Self {
            start: Bound::Included(start),
            end: Bound::Included(end),
        }
    }
}

/// Extensible trait implemented by all NOVA indexing engines.
pub trait Index: Send + Sync {
    /// Identifier name of the index (e.g. "idx_users_email").
    fn name(&self) -> &str;

    /// Document field indexed by this index (e.g. "email" or "profile.age").
    fn field(&self) -> &str;

    /// Category of index (Hash, Ordered, PrimaryKey).
    fn index_type(&self) -> IndexType;

    /// Insert a key-to-document mapping into the index.
    fn insert(&mut self, key: &Value, doc_id: &DocumentId) -> Result<(), IndexError>;

    /// Remove a key-to-document mapping from the index.
    fn remove(&mut self, key: &Value, doc_id: &DocumentId) -> Result<(), IndexError>;

    /// Look up all document IDs matching an exact key.
    fn lookup(&self, key: &Value) -> Vec<DocumentId>;

    /// Perform a range scan over the index (if supported).
    fn scan(&self, range: &IndexRange) -> Result<Vec<DocumentId>, IndexError>;

    /// Number of distinct entries or indexed records.
    fn len(&self) -> usize;

    /// Check if index is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear all index entries.
    fn clear(&mut self);
}
