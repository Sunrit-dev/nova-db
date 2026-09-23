pub mod document;
pub mod error;
pub mod event;
pub mod value;

pub use document::{Document, DocumentId};
pub use error::{ErrorCode, NovaError, Result};
pub use event::{DataEvent, EventType};
pub use value::Value;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;
    use uuid::Uuid;

    #[test]
    fn test_value_types_and_ordering() {
        let v_null = Value::Null;
        let v_bool = Value::Bool(true);
        let v_int = Value::Int(42);
        let v_float = Value::Float(42.5);
        let v_str = Value::String("hello".to_string());

        assert!(v_null < v_bool);
        assert!(v_bool < v_int);
        assert!(v_int < v_float);
        assert!(v_float < v_str);

        // Int / Float equality & ordering
        assert_eq!(Value::Int(10), Value::Float(10.0));
        assert_eq!(Value::Int(10).cmp(&Value::Float(10.5)), Ordering::Less);
        assert_eq!(Value::Float(10.5).cmp(&Value::Int(10)), Ordering::Greater);
    }

    #[test]
    fn test_document_and_path_lookup() {
        let mut doc = Document::with_id("user_001");
        doc.insert("name", "Sunrit");
        doc.insert("age", 25);

        let mut profile = std::collections::BTreeMap::new();
        profile.insert(
            "role".to_string(),
            Value::String("Systems Engineer".to_string()),
        );
        profile.insert("location".to_string(), Value::String("Kolkata".to_string()));
        doc.insert("profile", Value::Object(profile));

        assert_eq!(doc.get("name"), Some(&Value::String("Sunrit".to_string())));
        assert_eq!(
            doc.get_path("profile.role"),
            Some(&Value::String("Systems Engineer".to_string()))
        );
        assert_eq!(doc.get_path("profile.nonexistent"), None);
    }

    #[test]
    fn test_data_event_creation() {
        let doc = Document::with_id("u1");
        let event = DataEvent::new(
            1,
            EventType::Create,
            "default",
            "users",
            doc.id.clone(),
            None,
            Some(doc),
            None,
        );

        assert_eq!(event.target_uri(), "nova://default/users/u1");
        assert!(event.matches_collection("default", "users"));
        assert!(!event.matches_collection("default", "orders"));
    }

    #[test]
    fn test_value_uuid_and_timestamp() {
        let u = Uuid::new_v4();
        let val_u = Value::Uuid(*u.as_bytes());
        assert_eq!(val_u.type_name(), "uuid");

        let ts = Value::now();
        assert_eq!(ts.type_name(), "timestamp");
    }
}
