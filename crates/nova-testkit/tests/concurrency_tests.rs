use nova_client::NovaClient;
use nova_core::document::Document;
use nova_server::{NovaServer, ServerConfig};
use tempfile::tempdir;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_high_concurrency_read_write() {
    let tmp = tempdir().unwrap();
    let config = ServerConfig {
        port: 17666,
        data_dir: tmp.path().to_path_buf(),
        ..Default::default()
    };

    let server = NovaServer::new(config).unwrap();
    let shutdown = server.shutdown_handle();

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let num_tasks = 15;
    let items_per_task = 20;
    let mut handles = Vec::new();

    for t in 0..num_tasks {
        let handle = tokio::spawn(async move {
            let client = NovaClient::connect("127.0.0.1:17666").await.unwrap();
            let coll = client.collection("concurrent_items");

            for i in 0..items_per_task {
                let id = format!("task_{t}_item_{i}");
                let mut doc = Document::with_id(&*id);
                doc.insert("task_id", t as i64);
                doc.insert("index", i as i64);
                coll.insert(doc).await.unwrap();

                // Query back
                let found = coll.find_by_id(&id).await.unwrap();
                assert!(found.is_some());
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.await.unwrap();
    }

    // Verify total count matches exactly
    let client = NovaClient::connect("127.0.0.1:17666").await.unwrap();
    let coll = client.collection("concurrent_items");
    let total = coll.count(None).await.unwrap();
    assert_eq!(total, num_tasks * items_per_task);

    let _ = shutdown.send(());
}
