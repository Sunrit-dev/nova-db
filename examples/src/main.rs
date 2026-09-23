use nova_client::NovaClient;
use nova_core::document::Document;
use nova_server::{NovaServer, ServerConfig};
use tempfile::tempdir;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== NOVA DB Example: Real-time Telemetry & Data Flows ===");

    let tmp = tempdir()?;
    let config = ServerConfig {
        port: 17700,
        data_dir: tmp.path().to_path_buf(),
        ..Default::default()
    };

    let server = NovaServer::new(config)?;
    let shutdown = server.shutdown_handle();

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let client = NovaClient::connect("127.0.0.1:17700").await?;
    let sensors = client.collection("sensors");

    // 1. Subscribe to critical alerts using WATCH
    println!("Subscribing to critical alerts (temperature > 75.0)...");
    let mut watch_stream = sensors.watch(Some("temperature > 75.0")).await?;

    tokio::spawn(async move {
        while let Some(event) = watch_stream.next().await {
            if let Some(doc) = event.after {
                let id = doc.id.clone();
                let temp = doc.get("temperature").unwrap();
                println!("  [ALERT STREAM] High temperature on {id}: {temp}°C!");
            }
        }
    });

    // 2. Ingest telemetry data
    println!("Ingesting sensor telemetry readings...");
    for i in 1..=5 {
        let mut doc = Document::with_id(format!("sensor_node_{i:02}"));
        doc.insert("location", format!("Zone-{}", (i % 2) + 1));
        doc.insert("temperature", 60.0 + (i as f64 * 4.5));
        doc.insert("battery", 90 - i);
        sensors.insert(doc).await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }

    // 3. Query analytics
    println!("\nExecuting analytics queries via NQL:");
    let hot_sensors = sensors.find("temperature > 75.0").await?;
    println!("Sensors exceeding 75°C: {}", hot_sensors.len());
    for s in hot_sensors {
        println!("  - {}: {:?}", s.id, s.fields);
    }

    let total = sensors.count(None).await?;
    println!("Total sensors registered: {}", total);

    let _ = shutdown.send(());
    println!("\nExample completed successfully!");
    Ok(())
}
