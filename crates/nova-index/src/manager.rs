use crate::traits::{Index, IndexError};
use nova_core::document::Document;
use std::collections::HashMap;

/// Manages multiple secondary indices for a collection.
pub struct IndexManager {
    indexes: HashMap<String, Box<dyn Index>>,
}

impl IndexManager {
    pub fn new() -> Self {
        Self {
            indexes: HashMap::new(),
        }
    }

    /// Register a new index.
    pub fn add_index(&mut self, index: Box<dyn Index>) {
        self.indexes.insert(index.name().to_string(), index);
    }

    /// Remove an index by name.
    pub fn drop_index(&mut self, name: &str) -> bool {
        self.indexes.remove(name).is_some()
    }

    /// Access an index by name.
    pub fn get(&self, name: &str) -> Option<&dyn Index> {
        self.indexes.get(name).map(|b| b.as_ref())
    }

    /// Find an index that indexes a specific field.
    pub fn find_index_for_field(&self, field: &str) -> Option<&dyn Index> {
        self.indexes
            .values()
            .find(|idx| idx.field() == field)
            .map(|b| b.as_ref())
    }

    /// Notify all indexes that a new document was inserted.
    pub fn on_insert(&mut self, doc: &Document) -> Result<(), IndexError> {
        for index in self.indexes.values_mut() {
            let field = index.field();
            if let Some(val) = doc.get_path(field).or_else(|| doc.fields.get(field)) {
                index.insert(val, &doc.id)?;
            }
        }
        Ok(())
    }

    /// Notify all indexes that a document was updated.
    pub fn on_update(&mut self, before: &Document, after: &Document) -> Result<(), IndexError> {
        for index in self.indexes.values_mut() {
            let field = index.field();
            let old_val = before.get_path(field).or_else(|| before.fields.get(field));
            let new_val = after.get_path(field).or_else(|| after.fields.get(field));

            if old_val != new_val {
                if let Some(val) = old_val {
                    index.remove(val, &before.id)?;
                }
                if let Some(val) = new_val {
                    index.insert(val, &after.id)?;
                }
            }
        }
        Ok(())
    }

    /// Notify all indexes that a document was removed.
    pub fn on_remove(&mut self, doc: &Document) -> Result<(), IndexError> {
        for index in self.indexes.values_mut() {
            let field = index.field();
            if let Some(val) = doc.get_path(field).or_else(|| doc.fields.get(field)) {
                index.remove(val, &doc.id)?;
            }
        }
        Ok(())
    }

    /// Rebuild all indices from an iterator over documents (e.g. after recovery or index creation).
    pub fn rebuild<'a>(
        &mut self,
        docs: impl Iterator<Item = &'a Document>,
    ) -> Result<(), IndexError> {
        for index in self.indexes.values_mut() {
            index.clear();
        }
        for doc in docs {
            self.on_insert(doc)?;
        }
        Ok(())
    }

    /// Get list of index descriptions (name, field, type, count).
    pub fn list_indexes(&self) -> Vec<IndexInfo> {
        self.indexes
            .values()
            .map(|idx| IndexInfo {
                name: idx.name().to_string(),
                field: idx.field().to_string(),
                index_type: format!("{:?}", idx.index_type()),
                entries: idx.len(),
            })
            .collect()
    }
}

impl Default for IndexManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Metadata description of an active index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexInfo {
    pub name: String,
    pub field: String,
    pub index_type: String,
    pub entries: usize,
}
