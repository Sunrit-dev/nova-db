use nova_client::NovaClient;
use nova_core::document::Document;
use nova_server::{NovaServer, ServerConfig};
use std::time::Instant;
use tempfile::tempdir;

fn calculate_percentiles(mut latencies: Vec<f64>) -> (f64, f64, f64) {
    if latencies.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = latencies[(latencies.len() as f64 * 0.50) as usize];
    let p95 = latencies[((latencies.len() as f64 * 0.95) as usize).min(latencies.len() - 1)];
    let p99 = latencies[((latencies.len() as f64 * 0.99) as usize).min(latencies.len() - 1)];
    (p50, p95, p99)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("==================================================");
    println!("  NOVA DB Micro-Benchmark & Latency Profiler");
    println!("==================================================");

    let tmp = tempdir()?;
    let config = ServerConfig {
        port: 17800,
        data_dir: tmp.path().to_path_buf(),
        sync_mode: "none".to_string(), // In-memory/OS buffered for pure engine latency profile
        ..Default::default()
    };

    let server = NovaServer::new(config)?;
    let shutdown = server.shutdown_handle();

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let client = NovaClient::connect("127.0.0.1:17800").await?;
    let users = client.collection("bench_users");

    let iterations = 1000;

    // 1. Sequential Insert
    println!("\n[1/4] Benchmarking Sequential Writes (INSERT)...");
    let mut write_latencies = Vec::with_capacity(iterations);
    let start_writes = Instant::now();

    for i in 0..iterations {
        let mut doc = Document::with_id(format!("u_{i}"));
        doc.insert("age", (20 + (i % 50)) as i64);
        doc.insert("score", 85.5);

        let t0 = Instant::now();
        users.insert(doc).await?;
        write_latencies.push(t0.elapsed().as_secs_f64() * 1000.0);
    }

    let write_elapsed = start_writes.elapsed();
    let write_throughput = iterations as f64 / write_elapsed.as_secs_f64();
    let (w_p50, w_p95, w_p99) = calculate_percentiles(write_latencies);

    println!("  Throughput:  {:.1} ops/sec", write_throughput);
    println!(
        "  Latency:     p50: {:.2}ms, p95: {:.2}ms, p99: {:.2}ms",
        w_p50, w_p95, w_p99
    );

    // 2. Point Lookups (FIND by ID)
    println!("\n[2/4] Benchmarking Point Lookups (FIND by ID)...");
    let mut read_latencies = Vec::with_capacity(iterations);
    let start_reads = Instant::now();

    for i in 0..iterations {
        let id = format!("u_{i}");
        let t0 = Instant::now();
        let doc = users.find_by_id(&id).await?;
        assert!(doc.is_some());
        read_latencies.push(t0.elapsed().as_secs_f64() * 1000.0);
    }

    let read_elapsed = start_reads.elapsed();
    let read_throughput = iterations as f64 / read_elapsed.as_secs_f64();
    let (r_p50, r_p95, r_p99) = calculate_percentiles(read_latencies);

    println!("  Throughput:  {:.1} ops/sec", read_throughput);
    println!(
        "  Latency:     p50: {:.2}ms, p95: {:.2}ms, p99: {:.2}ms",
        r_p50, r_p95, r_p99
    );

    // 3. Predicate Filter Queries (WHERE age > 30)
    println!("\n[3/4] Benchmarking Filtered Range Queries (FIND WHERE)...");
    let mut filter_latencies = Vec::with_capacity(100);
    let start_filter = Instant::now();

    for _ in 0..100 {
        let t0 = Instant::now();
        let docs = users.find("age > 30").await?;
        assert!(!docs.is_empty());
        filter_latencies.push(t0.elapsed().as_secs_f64() * 1000.0);
    }

    let filter_elapsed = start_filter.elapsed();
    let filter_throughput = 100.0 / filter_elapsed.as_secs_f64();
    let (f_p50, f_p95, f_p99) = calculate_percentiles(filter_latencies);

    println!("  Throughput:  {:.1} queries/sec", filter_throughput);
    println!(
        "  Latency:     p50: {:.2}ms, p95: {:.2}ms, p99: {:.2}ms",
        f_p50, f_p95, f_p99
    );

    // 4. In-place Updates (UPDATE SET)
    println!("\n[4/4] Benchmarking In-place Updates (UPDATE SET)...");
    let mut update_latencies = Vec::with_capacity(100);
    let start_updates = Instant::now();

    for i in 0..100 {
        let t0 = Instant::now();
        let aff = users
            .update(&format!("id == \"u_{i}\""), "score = 99.0")
            .await?;
        assert_eq!(aff, 1);
        update_latencies.push(t0.elapsed().as_secs_f64() * 1000.0);
    }

    let update_elapsed = start_updates.elapsed();
    let update_throughput = 100.0 / update_elapsed.as_secs_f64();
    let (u_p50, u_p95, u_p99) = calculate_percentiles(update_latencies);

    println!("  Throughput:  {:.1} updates/sec", update_throughput);
    println!(
        "  Latency:     p50: {:.2}ms, p95: {:.2}ms, p99: {:.2}ms",
        u_p50, u_p95, u_p99
    );

    let _ = shutdown.send(());
    println!("\nAll benchmark suites completed successfully.");
    Ok(())
}
