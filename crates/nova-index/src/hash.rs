use crate::traits::{Index, IndexError, IndexRange, IndexType};
use nova_core::document::DocumentId;
use nova_core::value::Value;
use std::collections::{HashMap, HashSet};

/// An in-memory hash index optimized for fast O(1) exact-match queries.
pub struct HashIndex {
    name: String,
    field: String,
    entries: HashMap<Value, HashSet<DocumentId>>,
}

impl HashIndex {
    pub fn new(name: impl Into<String>, field: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            field: field.into(),
            entries: HashMap::new(),
        }
    }
}

impl Index for HashIndex {
    fn name(&self) -> &str {
        &self.name
    }

    fn field(&self) -> &str {
        &self.field
    }

    fn index_type(&self) -> IndexType {
        IndexType::Hash
    }

    fn insert(&mut self, key: &Value, doc_id: &DocumentId) -> Result<(), IndexError> {
        self.entries
            .entry(key.clone())
            .or_default()
            .insert(doc_id.clone());
        Ok(())
    }

    fn remove(&mut self, key: &Value, doc_id: &DocumentId) -> Result<(), IndexError> {
        if let Some(set) = self.entries.get_mut(key) {
            set.remove(doc_id);
            if set.is_empty() {
                self.entries.remove(key);
            }
        }
        Ok(())
    }

    fn lookup(&self, key: &Value) -> Vec<DocumentId> {
        match self.entries.get(key) {
            Some(set) => set.iter().cloned().collect(),
            None => Vec::new(),
        }
    }

    fn scan(&self, _range: &IndexRange) -> Result<Vec<DocumentId>, IndexError> {
        Err(IndexError::ScanNotSupported(self.name.clone()))
    }

    fn len(&self) -> usize {
        self.entries.values().map(|s| s.len()).sum()
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}
