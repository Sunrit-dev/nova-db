//! Indexing engine for NOVA DB.

pub mod hash;
pub mod manager;
pub mod ordered;
pub mod traits;

pub use hash::HashIndex;
pub use manager::{IndexInfo, IndexManager};
pub use ordered::OrderedIndex;
pub use traits::{Index, IndexError, IndexRange, IndexType};

#[cfg(test)]
mod tests {
    use super::*;
    use nova_core::document::Document;
    use nova_core::value::Value;

    #[test]
    fn test_hash_index() {
        let mut idx = HashIndex::new("idx_email", "email");
        let id1 = "u1".into();
        let id2 = "u2".into();
        let id3 = "u3".into();

        idx.insert(&Value::String("alice@nova.dev".to_string()), &id1)
            .unwrap();
        idx.insert(&Value::String("bob@nova.dev".to_string()), &id2)
            .unwrap();
        idx.insert(&Value::String("alice@nova.dev".to_string()), &id3)
            .unwrap();

        let mut matches = idx.lookup(&Value::String("alice@nova.dev".to_string()));
        matches.sort();
        assert_eq!(matches, vec![id1.clone(), id3.clone()]);

        idx.remove(&Value::String("alice@nova.dev".to_string()), &id1)
            .unwrap();
        let matches_after = idx.lookup(&Value::String("alice@nova.dev".to_string()));
        assert_eq!(matches_after, vec![id3]);
    }

    #[test]
    fn test_ordered_index_range_scan() {
        let mut idx = OrderedIndex::new("idx_age", "age");
        let id1 = "u1".into(); // 20
        let id2 = "u2".into(); // 30
        let id3 = "u3".into(); // 40
        let id4 = "u4".into(); // 50

        idx.insert(&Value::Int(20), &id1).unwrap();
        idx.insert(&Value::Int(30), &id2).unwrap();
        idx.insert(&Value::Int(40), &id3).unwrap();
        idx.insert(&Value::Int(50), &id4).unwrap();

        // Range scan: 25 <= age <= 45
        let range = IndexRange::between_inclusive(Value::Int(25), Value::Int(45));
        let mut results = idx.scan(&range).unwrap();
        results.sort();
        assert_eq!(results, vec![id2.clone(), id3.clone()]);

        // Range scan: age >= 40
        let range_gte = IndexRange::from_inclusive(Value::Int(40));
        let mut results_gte = idx.scan(&range_gte).unwrap();
        results_gte.sort();
        assert_eq!(results_gte, vec![id3, id4]);
    }

    #[test]
    fn test_index_manager_lifecycle() {
        let mut mgr = IndexManager::new();
        mgr.add_index(Box::new(OrderedIndex::new("idx_score", "score")));

        let mut doc1 = Document::with_id("p1");
        doc1.insert("score", 85);
        mgr.on_insert(&doc1).unwrap();

        let mut doc2 = Document::with_id("p2");
        doc2.insert("score", 95);
        mgr.on_insert(&doc2).unwrap();

        {
            let idx = mgr.find_index_for_field("score").unwrap();
            let range = IndexRange::from_inclusive(Value::Int(90));
            let top = idx.scan(&range).unwrap();
            assert_eq!(top, vec!["p2".into()]);
        }

        // Update score of p1 to 99
        let mut doc1_updated = doc1.clone();
        doc1_updated.insert("score", 99);
        mgr.on_update(&doc1, &doc1_updated).unwrap();

        let range = IndexRange::from_inclusive(Value::Int(90));
        let top_updated = mgr
            .find_index_for_field("score")
            .unwrap()
            .scan(&range)
            .unwrap();
        assert_eq!(top_updated.len(), 2);
    }
}
