//! Testkit, crash simulator, and fault injection harness for NOVA DB.

pub mod fault;
pub mod harness;

pub use fault::FaultInjector;
pub use harness::TestHarness;

#[cfg(test)]
mod tests {
    use super::*;
    use nova_core::document::Document;
    use nova_core::value::Value;

    #[tokio::test]
    async fn test_harness_crash_and_restart_recovery() {
        let mut harness = TestHarness::new().await.unwrap();

        // 1. Insert documents
        {
            let client = harness.client().await.unwrap();
            let users = client.collection("crash_users");

            let mut d1 = Document::with_id("u_persist_1");
            d1.insert("val", 100);
            users.insert(d1).await.unwrap();

            let mut d2 = Document::with_id("u_persist_2");
            d2.insert("val", 200);
            users.insert(d2).await.unwrap();
        }

        // 2. Simulate sudden crash (abort task)
        harness.kill();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // 3. Restart server on the same data directory
        harness.start_server().await.unwrap();

        // 4. Verify durable recovery
        {
            let client = harness.client().await.unwrap();
            let users = client.collection("crash_users");

            let docs = users.find_all().await.unwrap();
            assert_eq!(docs.len(), 2);

            let d1 = users.find_by_id("u_persist_1").await.unwrap().unwrap();
            assert_eq!(d1.get("val"), Some(&Value::Int(100)));

            let d2 = users.find_by_id("u_persist_2").await.unwrap().unwrap();
            assert_eq!(d2.get("val"), Some(&Value::Int(200)));
        }

        harness.shutdown().await;
    }
}
