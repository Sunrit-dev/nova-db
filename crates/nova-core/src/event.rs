use crate::document::{Document, DocumentId};
use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Type of data mutation or system lifecycle event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventType {
    Create,
    Update,
    Delete,
    Expire,
    IndexCreated,
    IndexDropped,
    Snapshot,
}

impl EventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventType::Create => "CREATE",
            EventType::Update => "UPDATE",
            EventType::Delete => "DELETE",
            EventType::Expire => "EXPIRE",
            EventType::IndexCreated => "INDEX_CREATED",
            EventType::IndexDropped => "INDEX_DROPPED",
            EventType::Snapshot => "SNAPSHOT",
        }
    }
}

/// A first-class structured event representing a mutation or change in the data flow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataEvent {
    /// Monotonically increasing sequence number for this event.
    pub sequence: u64,
    /// Type of mutation event.
    pub event_type: EventType,
    /// Namespace (e.g. "default", "production", "analytics").
    pub namespace: String,
    /// Collection name (e.g. "users", "metrics").
    pub collection: String,
    /// Document identifier affected.
    pub document_id: DocumentId,
    /// State of the document before this mutation (None on Create).
    pub before: Option<Document>,
    /// State of the document after this mutation (None on Delete).
    pub after: Option<Document>,
    /// Timestamp in microseconds UTC.
    pub timestamp: i64,
    /// Optional transaction ID if part of an atomic transaction.
    pub tx_id: Option<u64>,
}

impl DataEvent {
    /// Create a new DataEvent with current timestamp.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sequence: u64,
        event_type: EventType,
        namespace: impl Into<String>,
        collection: impl Into<String>,
        document_id: DocumentId,
        before: Option<Document>,
        after: Option<Document>,
        tx_id: Option<u64>,
    ) -> Self {
        Self {
            sequence,
            event_type,
            namespace: namespace.into(),
            collection: collection.into(),
            document_id,
            before,
            after,
            timestamp: Utc::now().timestamp_micros(),
            tx_id,
        }
    }

    /// Helper to get the canonical path: "namespace://collection/document_id"
    pub fn target_uri(&self) -> String {
        format!(
            "nova://{}/{}/{}",
            self.namespace, self.collection, self.document_id
        )
    }

    /// Check if event matches a namespace and collection.
    pub fn matches_collection(&self, ns: &str, coll: &str) -> bool {
        self.namespace == ns && self.collection == coll
    }
}
