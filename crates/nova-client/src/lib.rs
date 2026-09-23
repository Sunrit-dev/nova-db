//! Official asynchronous Rust client for NOVA DB.

pub mod client;
pub mod collection;
pub mod stream;

pub use client::NovaClient;
pub use collection::CollectionHandle;
pub use stream::EventStream;

#[cfg(test)]
mod tests {
    use super::*;
    use nova_core::document::Document;
    use nova_core::event::EventType;
    use nova_core::value::Value;
    use nova_server::{NovaServer, ServerConfig};
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_client_server_e2e_and_watch() {
        let tmp = tempdir().unwrap();
        let config = ServerConfig {
            port: 17401,
            data_dir: tmp.path().to_path_buf(),
            ..Default::default()
        };

        let server = NovaServer::new(config).unwrap();
        let shutdown = server.shutdown_handle();

        // Spawn server in background
        tokio::spawn(async move {
            let _ = server.run().await;
        });

        // Wait brief moment for listener to bind
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Connect client
        let client = NovaClient::connect("127.0.0.1:17401").await.unwrap();
        client.ping().await.unwrap();

        let users = client.collection("users");

        // 1. Subscribe to WATCH stream before inserting
        let mut watch_stream = users.watch(None).await.unwrap();

        // 2. Insert document
        let mut doc = Document::with_id("u_client_1");
        doc.insert("name", "Deepak");
        doc.insert("city", "Bangalore");
        let inserted = users.insert(doc).await.unwrap();
        assert_eq!(inserted.id.as_str(), "u_client_1");

        // 3. Verify event received in WATCH stream
        let event = tokio::time::timeout(tokio::time::Duration::from_secs(2), watch_stream.next())
            .await
            .expect("timeout waiting for watch event")
            .expect("event should not be None");

        assert_eq!(event.event_type, EventType::Create);
        assert_eq!(event.document_id.as_str(), "u_client_1");

        // 4. Find document
        let found = users.find("city == \"Bangalore\"").await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].get("name"),
            Some(&Value::String("Deepak".to_string()))
        );

        // 5. Update document
        let updated_count = users
            .update("id == \"u_client_1\"", "role = \"senior\"")
            .await
            .unwrap();
        assert_eq!(updated_count, 1);

        // 6. Remove document
        let removed_count = users.remove("id == \"u_client_1\"").await.unwrap();
        assert_eq!(removed_count, 1);

        // Graceful server shutdown
        let _ = shutdown.send(());
    }
}
