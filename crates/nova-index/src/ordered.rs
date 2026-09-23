use crate::traits::{Index, IndexError, IndexRange, IndexType};
use nova_core::document::DocumentId;
use nova_core::value::Value;
use std::collections::{BTreeMap, BTreeSet};

/// A ordered B-Tree index supporting exact match and range scans.
pub struct OrderedIndex {
    name: String,
    field: String,
    entries: BTreeMap<Value, BTreeSet<DocumentId>>,
}

impl OrderedIndex {
    pub fn new(name: impl Into<String>, field: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            field: field.into(),
            entries: BTreeMap::new(),
        }
    }
}

impl Index for OrderedIndex {
    fn name(&self) -> &str {
        &self.name
    }

    fn field(&self) -> &str {
        &self.field
    }

    fn index_type(&self) -> IndexType {
        IndexType::Ordered
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

    fn scan(&self, range: &IndexRange) -> Result<Vec<DocumentId>, IndexError> {
        let mut results = Vec::new();

        let sub_range = match (&range.start, &range.end) {
            (std::ops::Bound::Unbounded, std::ops::Bound::Unbounded) => self.entries.range(..),
            (std::ops::Bound::Included(s), std::ops::Bound::Unbounded) => self.entries.range(s..),
            (std::ops::Bound::Excluded(s), std::ops::Bound::Unbounded) => self
                .entries
                .range((std::ops::Bound::Excluded(s), std::ops::Bound::Unbounded)),
            (std::ops::Bound::Unbounded, std::ops::Bound::Included(e)) => self.entries.range(..=e),
            (std::ops::Bound::Unbounded, std::ops::Bound::Excluded(e)) => self.entries.range(..e),
            (std::ops::Bound::Included(s), std::ops::Bound::Included(e)) => {
                self.entries.range(s..=e)
            }
            (std::ops::Bound::Included(s), std::ops::Bound::Excluded(e)) => {
                self.entries.range(s..e)
            }
            (std::ops::Bound::Excluded(s), std::ops::Bound::Included(e)) => self
                .entries
                .range((std::ops::Bound::Excluded(s), std::ops::Bound::Included(e))),
            (std::ops::Bound::Excluded(s), std::ops::Bound::Excluded(e)) => self
                .entries
                .range((std::ops::Bound::Excluded(s), std::ops::Bound::Excluded(e))),
        };

        for (_key, doc_set) in sub_range {
            for doc_id in doc_set {
                results.push(doc_id.clone());
            }
        }

        Ok(results)
    }

    fn len(&self) -> usize {
        self.entries.values().map(|s| s.len()).sum()
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}
