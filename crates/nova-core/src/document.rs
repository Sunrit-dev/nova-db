use crate::error::{NovaError, Result};
use crate::value::Value;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use uuid::Uuid;

/// Unique identifier for a document within a collection.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DocumentId(String);

impl DocumentId {
    /// Create a new DocumentId from a string, validating format.
    pub fn new(id: impl Into<String>) -> Result<Self> {
        let s = id.into();
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(NovaError::invalid_query("Document ID cannot be empty"));
        }
        if trimmed.len() > 256 {
            return Err(NovaError::invalid_query(
                "Document ID cannot exceed 256 bytes",
            ));
        }
        Ok(DocumentId(trimmed.to_string()))
    }

    /// Generate a new unique v4 UUID-based DocumentId.
    pub fn generate() -> Self {
        DocumentId(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DocumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for DocumentId {
    fn from(s: &str) -> Self {
        DocumentId::new(s).unwrap_or_else(|_| DocumentId::generate())
    }
}

impl From<String> for DocumentId {
    fn from(s: String) -> Self {
        DocumentId::new(s).unwrap_or_else(|_| DocumentId::generate())
    }
}

/// A structured document stored in NOVA DB.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub id: DocumentId,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub fields: BTreeMap<String, Value>,
}

impl Document {
    /// Create a new document with an auto-generated ID.
    pub fn new() -> Self {
        let now = Utc::now().timestamp_micros();
        Self {
            id: DocumentId::generate(),
            version: 1,
            created_at: now,
            updated_at: now,
            fields: BTreeMap::new(),
        }
    }

    /// Create a new document with a specified ID.
    pub fn with_id(id: impl Into<DocumentId>) -> Self {
        let now = Utc::now().timestamp_micros();
        Self {
            id: id.into(),
            version: 1,
            created_at: now,
            updated_at: now,
            fields: BTreeMap::new(),
        }
    }

    /// Retrieve a field value by key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        if key == "_id" || key == "id" {
            // Note: will be matched if explicitly queried
            return None;
        }
        self.fields.get(key)
    }

    /// Retrieve a field value by nested dot-path (e.g. "profile.email").
    pub fn get_path(&self, path: &str) -> Option<&Value> {
        if path == "_id" || path == "id" {
            return None;
        }
        let parts: Vec<&str> = path.split('.').collect();
        if parts.is_empty() {
            return None;
        }
        let mut current = self.fields.get(parts[0])?;
        for part in &parts[1..] {
            match current {
                Value::Object(map) => {
                    current = map.get(*part)?;
                }
                _ => return None,
            }
        }
        Some(current)
    }

    /// Set a field in the document.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) -> Option<Value> {
        self.fields.insert(key.into(), value.into())
    }

    /// Remove a field from the document.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.fields.remove(key)
    }

    /// Advance document revision and update timestamp.
    pub fn advance_version(&mut self) {
        self.version += 1;
        self.updated_at = Utc::now().timestamp_micros();
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper macro for constructing documents idiomatically.
#[macro_export]
macro_rules! doc {
    ($($key:expr => $val:expr),* $(,)?) => {{
        let mut d = $crate::document::Document::new();
        $(
            d.insert($key, $val);
        )*
        d
    }};
}
