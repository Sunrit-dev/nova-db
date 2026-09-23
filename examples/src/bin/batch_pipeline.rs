//! High-throughput batch ingestion and pipeline execution example for NOVA DB.

use nova_client::NovaClient;
use nova_core::document::Document;
use nova_server::{NovaServer, ServerConfig};
use std::time::Instant;
use tempfile::tempdir;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== NOVA DB: High-Throughput Batch Mutation Pipeline ===");

    let tmp = tempdir()?;
    let config = ServerConfig {
        port: 17701,
        data_dir: tmp.path().to_path_buf(),
        ..Default::default()
    };

    let server = NovaServer::new(config)?;
    let shutdown = server.shutdown_handle();
    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(60)).await;

    let client = NovaClient::connect("127.0.0.1:17701").await?;
    let orders = client.collection("orders");

    println!("Batch inserting 100 customer order records...");
    let start = Instant::now();

    for i in 1..=100 {
        let mut order = Document::with_id(format!("ord_{i:04}"));
        order.insert("customer_id", format!("cust_{}", i % 10));
        order.insert("amount", (i as f64) * 14.5);
        order.insert("status", if i % 5 == 0 { "shipped" } else { "pending" });
        orders.insert(order).await?;
    }

    let elapsed = start.elapsed();
    let ops_sec = 100.0 / elapsed.as_secs_f64();
    println!("✓ Ingested 100 orders in {elapsed:.2?} ({ops_sec:.1} ops/sec)");

    // Query high-value pending orders
    let pending_high_value = orders.find("amount > 500.0 AND status == \"pending\"").await?;
    println!("High-value pending orders found: {}", pending_high_value.len());

    let _ = shutdown.send(());
    println!("Pipeline demo finished cleanly.");
    Ok(())
}
