//! Asynchronous server, query executor, and event pipeline for NOVA DB.

pub mod config;
pub mod connection;
pub mod engine;
pub mod metrics;
pub mod server;
pub mod web;

pub use config::ServerConfig;
pub use connection::ConnectionHandler;
pub use engine::DatabaseEngine;
pub use metrics::{MetricsSnapshot, ServerMetrics};
pub use server::NovaServer;
pub use web::run_web_studio;

#[cfg(test)]
mod tests {
    use super::*;
    use nova_core::value::Value;
    use nova_protocol::ResponsePayload;
    use nova_query::parse_nql;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_engine_query_execution_lifecycle() {
        let tmp = tempdir().unwrap();
        let config = ServerConfig {
            data_dir: tmp.path().to_path_buf(),
            ..Default::default()
        };

        let metrics = std::sync::Arc::new(ServerMetrics::new());
        let engine = DatabaseEngine::open(config, metrics).unwrap();

        // 1. Insert documents via NQL
        let insert_nql =
            r#"INSERT INTO users VALUES ({ "name": "Sunrit", "role": "engineer", "age": 28 })"#;
        let stmt1 = parse_nql(insert_nql).unwrap();
        let resp1 = engine.execute(stmt1, "default").await.unwrap();
        match resp1 {
            ResponsePayload::Records { documents, count } => {
                assert_eq!(count, 1);
                assert_eq!(
                    documents[0].get("name"),
                    Some(&Value::String("Sunrit".to_string()))
                );
            }
            _ => panic!("Expected Records response"),
        }

        // 2. Query document via FIND
        let find_nql = "FIND users WHERE age > 20";
        let stmt2 = parse_nql(find_nql).unwrap();
        let resp2 = engine.execute(stmt2, "default").await.unwrap();
        match resp2 {
            ResponsePayload::Records { documents, count } => {
                assert_eq!(count, 1);
                assert_eq!(
                    documents[0].get("role"),
                    Some(&Value::String("engineer".to_string()))
                );
            }
            _ => panic!("Expected Records response"),
        }

        // 3. Update document via UPDATE
        let update_nql = r#"UPDATE users SET role = "architect" WHERE age == 28"#;
        let stmt3 = parse_nql(update_nql).unwrap();
        let resp3 = engine.execute(stmt3, "default").await.unwrap();
        match resp3 {
            ResponsePayload::Success { affected, .. } => {
                assert_eq!(affected, 1);
            }
            _ => panic!("Expected Success response"),
        }

        // 4. Verify updated document
        let find_updated = "FIND users WHERE role == \"architect\"";
        let stmt4 = parse_nql(find_updated).unwrap();
        let resp4 = engine.execute(stmt4, "default").await.unwrap();
        match resp4 {
            ResponsePayload::Records { count, .. } => {
                assert_eq!(count, 1);
            }
            _ => panic!("Expected Records response"),
        }

        // 5. Remove document
        let remove_nql = "REMOVE users WHERE age == 28";
        let stmt5 = parse_nql(remove_nql).unwrap();
        let resp5 = engine.execute(stmt5, "default").await.unwrap();
        match resp5 {
            ResponsePayload::Success { affected, .. } => {
                assert_eq!(affected, 1);
            }
            _ => panic!("Expected Success response"),
        }

        // 6. Verify removed
        let count_nql = "COUNT users";
        let stmt6 = parse_nql(count_nql).unwrap();
        let resp6 = engine.execute(stmt6, "default").await.unwrap();
        assert_eq!(resp6, ResponsePayload::Count(0));
    }
}
