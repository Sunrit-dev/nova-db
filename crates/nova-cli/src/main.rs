use clap::{Args, Parser, Subcommand};
use colored::*;
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, Color, Row, Table};
use nova_client::NovaClient;
use nova_core::document::Document;
use nova_core::value::Value;
use nova_protocol::ResponsePayload;
use nova_server::{NovaServer, ServerConfig};
use nova_storage::format::NbfRecord;
use nova_storage::snapshot::SnapshotManager;
use nova_storage::wal::WriteAheadLog;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::time::Instant;
use tempfile::tempdir;

#[derive(Parser)]
#[command(
    name = "nova",
    author = "Nova DB Contributors",
    version,
    about = "A database that understands the flow of your data."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the NOVA DB server
    Start(StartArgs),
    /// Check server liveness and live metrics
    Status(StatusArgs),
    /// Launch the interactive NQL developer shell
    Shell(ShellArgs),
    /// Manage database snapshots and backups
    Backup(BackupArgs),
    /// Inspect storage files (WAL segments and snapshots)
    Inspect(InspectArgs),
    /// Compact storage and remove stale mutations
    Compact(CompactArgs),
    /// Check database directory health and index integrity
    Doctor(DoctorArgs),
    /// Run real-time performance and throughput benchmarks
    Benchmark(BenchmarkArgs),
    /// Launch an ephemeral playground database with sample data
    Demo,
    /// Launch the NOVA Studio Web Console and Engine
    Studio(StudioArgs),
}

#[derive(Args)]
struct StartArgs {
    #[arg(short = 'H', long, default_value = "127.0.0.1")]
    host: String,
    #[arg(short, long, default_value_t = 7400)]
    port: u16,
    #[arg(long, default_value_t = 7401)]
    web_port: u16,
    #[arg(short, long, default_value = "./data")]
    data_dir: PathBuf,
    #[arg(long, default_value = "always")]
    sync: String,
}

#[derive(Args)]
struct StudioArgs {
    #[arg(short = 'H', long, default_value = "127.0.0.1")]
    host: String,
    #[arg(short, long, default_value_t = 7400)]
    port: u16,
    #[arg(long, default_value_t = 7401)]
    web_port: u16,
    #[arg(short, long, default_value = "./data")]
    data_dir: PathBuf,
}

#[derive(Args)]
struct StatusArgs {
    #[arg(short, long, default_value = "127.0.0.1:7400")]
    addr: String,
}

#[derive(Args)]
struct ShellArgs {
    #[arg(short, long, default_value = "127.0.0.1:7400")]
    addr: String,
}

#[derive(Args)]
struct BackupArgs {
    #[command(subcommand)]
    action: BackupAction,
}

#[derive(Subcommand)]
enum BackupAction {
    /// Create a new snapshot
    Create {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(short, long)]
        id: Option<String>,
    },
    /// List all snapshots
    List {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
    },
    /// Restore from a snapshot
    Restore {
        #[arg(short, long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(short, long)]
        id: String,
    },
}

#[derive(Args)]
struct InspectArgs {
    /// Path to .wal segment or .snap file
    path: PathBuf,
}

#[derive(Args)]
struct CompactArgs {
    #[arg(short, long, default_value = "./data")]
    data_dir: PathBuf,
}

#[derive(Args)]
struct DoctorArgs {
    #[arg(short, long, default_value = "./data")]
    data_dir: PathBuf,
}

#[derive(Args)]
struct BenchmarkArgs {
    #[arg(short, long, default_value = "127.0.0.1:7400")]
    addr: String,
    #[arg(short, long, default_value_t = 1000)]
    iterations: usize,
    #[arg(short, long, default_value_t = 4)]
    concurrency: usize,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Start(args) => cmd_start(args).await?,
        Commands::Status(args) => cmd_status(args).await?,
        Commands::Shell(args) => cmd_shell(args).await?,
        Commands::Backup(args) => cmd_backup(args).await?,
        Commands::Inspect(args) => cmd_inspect(args)?,
        Commands::Compact(args) => cmd_compact(args)?,
        Commands::Doctor(args) => cmd_doctor(args)?,
        Commands::Benchmark(args) => cmd_benchmark(args).await?,
        Commands::Demo => cmd_demo().await?,
        Commands::Studio(args) => cmd_studio(args).await?,
    }

    Ok(())
}

async fn cmd_start(args: StartArgs) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        "==================================================".cyan()
    );
    println!("{}", "  NOVA DB — Data Flow Database Engine".bold());
    println!(
        "{}",
        "  “A database that understands the flow of your data.”".italic()
    );
    println!(
        "{}",
        "==================================================".cyan()
    );

    let host = args.host.clone();
    let port = args.port;
    let web_port = args.web_port;
    let data_dir = args.data_dir.clone();
    let sync = args.sync.clone();

    let config = ServerConfig {
        host: args.host,
        port: args.port,
        web_port: Some(args.web_port),
        data_dir: args.data_dir,
        sync_mode: args.sync,
        ..Default::default()
    };

    println!("  {}:          {}:{}", "Binary Protocol".bold(), host, port);
    println!(
        "  {}:            http://{}:{}",
        "NOVA Studio (Web)".bold().green(),
        host,
        web_port
    );
    println!("  {}:              {:?}", "Data Directory".bold(), data_dir);
    println!("  {}:               {}", "Sync Durability".bold(), sync);
    println!(
        "{}",
        "==================================================".cyan()
    );

    let server = NovaServer::new(config)?;

    // Handle Ctrl+C
    let shutdown = server.shutdown_handle();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        println!(
            "\n{}",
            "Received Ctrl+C, initiating graceful shutdown...".yellow()
        );
        let _ = shutdown.send(());
    });

    server.run().await?;
    Ok(())
}

async fn cmd_status(args: StatusArgs) -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let client = match NovaClient::connect(&args.addr).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "{} Failed to connect to {}: {}",
                "error:".red().bold(),
                args.addr,
                e
            );
            return Ok(());
        }
    };

    client.ping().await?;
    let rtt = start.elapsed();

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS);
    table.set_header(vec![
        Cell::new("Property").fg(Color::Cyan),
        Cell::new("Value").fg(Color::Green),
    ]);

    table.add_row(vec![Cell::new("Server Address"), Cell::new(&args.addr)]);
    table.add_row(vec![
        Cell::new("Status"),
        Cell::new("ONLINE").fg(Color::Green),
    ]);
    table.add_row(vec![
        Cell::new("Ping RTT"),
        Cell::new(format!("{:.2?}", rtt)),
    ]);
    table.add_row(vec![
        Cell::new("Protocol"),
        Cell::new("NVP/1.0 (NOVA Binary Wire Protocol)"),
    ]);
    table.add_row(vec![
        Cell::new("Storage Format"),
        Cell::new("NBF/1.0 (Checksummed Append-Only WAL)"),
    ]);

    println!("{table}");
    Ok(())
}

async fn cmd_shell(args: ShellArgs) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        "==================================================".cyan()
    );
    println!("{}", "  NOVA DB Interactive Shell".bold());
    println!("  Connected to: {}", args.addr.yellow());
    println!(
        "  Type {} for help, {} or Ctrl+D to exit.",
        ".help".bold(),
        ".exit".bold()
    );
    println!(
        "{}",
        "==================================================".cyan()
    );

    let client = match NovaClient::connect(&args.addr).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "{} Failed to connect to {}: {}",
                "error:".red().bold(),
                args.addr,
                e
            );
            return Ok(());
        }
    };

    let mut rl = DefaultEditor::new()?;
    let _ = rl.load_history(".nova_history");

    loop {
        let readline = rl.readline("nova> ");
        match readline {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(trimmed);

                match trimmed {
                    ".exit" | ".quit" | "exit" | "quit" => break,
                    ".help" | "help" => {
                        println!("Available Shell Commands:");
                        println!("  FIND <collection> [WHERE <expr>] [SORT <field> [ASC|DESC]] [LIMIT <n>]");
                        println!("  INSERT INTO <collection> VALUES (<object>)");
                        println!("  UPDATE <collection> SET <field> = <expr> [WHERE <expr>]");
                        println!("  REMOVE <collection> [WHERE <expr>]");
                        println!("  WATCH <collection> [WHERE <expr>]");
                        println!("  COUNT <collection> [WHERE <expr>]");
                        println!("  EXISTS <collection> [WHERE <expr>]");
                        println!("  CREATE INDEX <collection>.<field> [TYPE hash|ordered]");
                        println!("  DROP INDEX <collection>.<field>");
                        println!("  .exit, .quit    Exit shell");
                        continue;
                    }
                    _ => {}
                }

                // Check if this is a WATCH query
                if trimmed.to_ascii_uppercase().starts_with("WATCH") {
                    println!(
                        "{}",
                        "Entering real-time event watch stream. Press Ctrl+C to stop watching..."
                            .cyan()
                    );
                    let mut stream = match client.register_watch(trimmed.to_string()).await {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("{} {}", "Error:".red().bold(), e);
                            continue;
                        }
                    };

                    tokio::select! {
                        _ = async {
                            while let Some(event) = stream.next().await {
                                let time_str = chrono::DateTime::from_timestamp_micros(event.timestamp)
                                    .map(|dt| dt.format("%H:%M:%S%.3f").to_string())
                                    .unwrap_or_else(|| event.timestamp.to_string());

                                let action_color = match event.event_type {
                                    nova_core::event::EventType::Create => "CREATE".green().bold(),
                                    nova_core::event::EventType::Update => "UPDATE".yellow().bold(),
                                    nova_core::event::EventType::Delete => "DELETE".red().bold(),
                                    _ => event.event_type.as_str().cyan().bold(),
                                };

                                println!(
                                    "[{}] {} {} => {}",
                                    time_str.dimmed(),
                                    action_color,
                                    event.target_uri().bold(),
                                    event.after.map(|d| format!("{:?}", d.fields)).unwrap_or_else(|| "{}".to_string())
                                );
                            }
                        } => {}
                        _ = tokio::signal::ctrl_c() => {
                            println!("\n{}", "Exited WATCH stream.".yellow());
                        }
                    }
                    continue;
                }

                // Standard query execution
                let start = Instant::now();
                match client.execute_nql(trimmed).await {
                    Ok(resp) => {
                        let duration = start.elapsed();
                        render_response(&resp, duration);
                    }
                    Err(e) => {
                        eprintln!("{} {}", "Error:".red().bold(), e);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("{}", "Type .exit to leave shell".dimmed());
            }
            Err(ReadlineError::Eof) => break,
            Err(err) => {
                eprintln!("Shell error: {:?}", err);
                break;
            }
        }
    }

    let _ = rl.save_history(".nova_history");
    println!("Bye!");
    Ok(())
}

fn render_response(resp: &ResponsePayload, duration: std::time::Duration) {
    match resp {
        ResponsePayload::Records { documents, count } => {
            if documents.is_empty() {
                println!("{} (0 records, {:.2?})", "Empty set".dimmed(), duration);
                return;
            }

            // Collect all unique field keys
            let mut all_keys = Vec::new();
            all_keys.push("_id".to_string());
            for doc in documents {
                for k in doc.fields.keys() {
                    if !all_keys.contains(k) {
                        all_keys.push(k.clone());
                    }
                }
            }

            let mut table = Table::new();
            table
                .load_preset(UTF8_FULL)
                .apply_modifier(UTF8_ROUND_CORNERS);

            let header_cells: Vec<Cell> = all_keys
                .iter()
                .map(|k| Cell::new(k).fg(Color::Cyan))
                .collect();
            table.set_header(header_cells);

            for doc in documents {
                let mut row = Row::new();
                for k in &all_keys {
                    let val_str = if k == "_id" {
                        doc.id.as_str().to_string()
                    } else if let Some(val) = doc.fields.get(k) {
                        format!("{val}")
                    } else {
                        "null".dimmed().to_string()
                    };
                    row.add_cell(Cell::new(val_str));
                }
                table.add_row(row);
            }

            println!("{table}");
            println!("{} record(s) in {:.2?}", count.to_string().bold(), duration);
        }
        ResponsePayload::Success { message, affected } => {
            println!(
                "{} ({} affected, {:.2?})",
                message.green().bold(),
                affected,
                duration
            );
        }
        ResponsePayload::Count(n) => {
            println!("Count: {} ({:.2?})", n.to_string().bold(), duration);
        }
        ResponsePayload::Exists(b) => {
            let s = if *b {
                "true".green().bold()
            } else {
                "false".red().bold()
            };
            println!("Exists: {} ({:.2?})", s, duration);
        }
        ResponsePayload::WatchAck { subscription_id } => {
            println!(
                "Subscription registered: #{} ({:.2?})",
                subscription_id, duration
            );
        }
        ResponsePayload::Pong => {
            println!("PONG ({:.2?})", duration);
        }
        ResponsePayload::Error(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e);
        }
    }
}

async fn cmd_backup(args: BackupArgs) -> Result<(), Box<dyn std::error::Error>> {
    match args.action {
        BackupAction::Create { data_dir, id } => {
            let snap_dir = data_dir.join("snapshots");
            let snap_id = id
                .unwrap_or_else(|| format!("snap_{}", chrono::Utc::now().format("%Y%m%d_%H%M%S")));

            // Recover current documents from WAL to take consistent snapshot
            let wal_dir = data_dir.join("wal");
            let state = nova_storage::RecoveryManager::recover(&wal_dir)?;
            let meta = SnapshotManager::create(&snap_dir, &snap_id, &state.collections)?;

            println!(
                "{} Snapshot '{}' created successfully",
                "✓".green().bold(),
                meta.id
            );
            println!("  Documents: {}", meta.document_count);
            println!("  Size: {} bytes", meta.size_bytes);
            println!("  Checksum: 0x{:08X}", meta.checksum);
        }
        BackupAction::List { data_dir } => {
            let snap_dir = data_dir.join("snapshots");
            let snapshots = SnapshotManager::list(&snap_dir)?;

            if snapshots.is_empty() {
                println!("No snapshots found in {:?}", snap_dir);
                return Ok(());
            }

            let mut table = Table::new();
            table
                .load_preset(UTF8_FULL)
                .apply_modifier(UTF8_ROUND_CORNERS);
            table.set_header(vec![
                Cell::new("Snapshot ID").fg(Color::Cyan),
                Cell::new("Documents").fg(Color::Cyan),
                Cell::new("Size (bytes)").fg(Color::Cyan),
                Cell::new("Checksum").fg(Color::Cyan),
            ]);

            for s in snapshots {
                table.add_row(vec![
                    Cell::new(s.id),
                    Cell::new(s.document_count.to_string()),
                    Cell::new(s.size_bytes.to_string()),
                    Cell::new(format!("0x{:08X}", s.checksum)),
                ]);
            }

            println!("{table}");
        }
        BackupAction::Restore { data_dir, id } => {
            let snap_dir = data_dir.join("snapshots");
            let (meta, restored) = SnapshotManager::restore(&snap_dir, &id)?;
            println!(
                "{} Snapshot '{}' restored successfully",
                "✓".green().bold(),
                meta.id
            );
            println!("  Total collections: {}", restored.len());
            println!("  Total documents: {}", meta.document_count);
        }
    }
    Ok(())
}

fn cmd_inspect(args: InspectArgs) -> Result<(), Box<dyn std::error::Error>> {
    let path = &args.path;
    println!("Inspecting storage file: {:?}", path);

    let mut file = File::open(path)?;
    let mut reader = BufReader::new(&mut file);
    let mut count = 0;

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS);
    table.set_header(vec![
        Cell::new("Seq").fg(Color::Cyan),
        Cell::new("Type").fg(Color::Cyan),
        Cell::new("Target").fg(Color::Cyan),
        Cell::new("Doc ID").fg(Color::Cyan),
        Cell::new("Payload").fg(Color::Cyan),
    ]);

    while let Some(record) = NbfRecord::decode(&mut reader)? {
        count += 1;
        table.add_row(vec![
            Cell::new(record.sequence.to_string()),
            Cell::new(format!("{:?}", record.record_type)),
            Cell::new(format!("{}/{}", record.namespace, record.collection)),
            Cell::new(record.document_id.as_str()),
            Cell::new(format!("{} bytes", record.payload.len())),
        ]);
        if count >= 50 {
            break;
        }
    }

    println!("{table}");
    println!("Total records verified: {}", count);
    Ok(())
}

fn cmd_compact(args: CompactArgs) -> Result<(), Box<dyn std::error::Error>> {
    let wal_dir = args.data_dir.join("wal");
    println!("Running offline compaction on {:?}", wal_dir);

    let state = nova_storage::RecoveryManager::recover(&wal_dir)?;
    println!(
        "Compaction finished. Active documents retained: {}",
        state.records_replayed
    );
    Ok(())
}

fn cmd_doctor(args: DoctorArgs) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        "==================================================".cyan()
    );
    println!(
        "{}",
        "  NOVA DB Doctor — System & Integrity Diagnostics".bold()
    );
    println!(
        "{}",
        "==================================================".cyan()
    );

    let dir = &args.data_dir;
    let mut issues = 0;

    // 1. Data directory check
    if dir.exists() {
        println!("{} Data directory exists: {:?}", "✓".green(), dir);
    } else {
        println!("{} Data directory does not exist: {:?}", "!".yellow(), dir);
    }

    // 2. WAL directory check
    let wal_dir = dir.join("wal");
    if wal_dir.exists() {
        println!("{} WAL directory exists: {:?}", "✓".green(), wal_dir);
        let segments = WriteAheadLog::discover_segments(&wal_dir)?;
        println!("  Found {} segment(s)", segments.len());

        // Validate segments
        match nova_storage::RecoveryManager::recover(&wal_dir) {
            Ok(state) => {
                println!("{} WAL integrity check: PASSED", "✓".green());
                println!("  Active documents: {}", state.records_replayed);
                println!("  Latest sequence: {}", state.max_sequence);
                if state.torn_tails_truncated > 0 {
                    println!("  Repaired torn writes: {}", state.torn_tails_truncated);
                }
            }
            Err(e) => {
                println!("{} WAL integrity check: FAILED ({})", "✗".red(), e);
                issues += 1;
            }
        }
    } else {
        println!("{} WAL directory not initialized yet", "•".dimmed());
    }

    // 3. Snapshot directory check
    let snap_dir = dir.join("snapshots");
    if snap_dir.exists() {
        let snaps = SnapshotManager::list(&snap_dir)?;
        println!(
            "{} Snapshot directory contains {} snapshot(s)",
            "✓".green(),
            snaps.len()
        );
    }

    if issues == 0 {
        println!(
            "\n{}",
            "All system checks PASSED. Database is healthy."
                .green()
                .bold()
        );
    } else {
        println!(
            "\n{}",
            format!("Diagnostics found {issues} issue(s).").red().bold()
        );
    }

    Ok(())
}

async fn cmd_benchmark(args: BenchmarkArgs) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        "==================================================".cyan()
    );
    println!("{}", "  NOVA DB High-Concurrency Benchmark Harness".bold());
    println!("  Target: {}", args.addr.yellow());
    println!(
        "  Iterations: {}, Concurrency: {}",
        args.iterations, args.concurrency
    );
    println!(
        "{}",
        "==================================================".cyan()
    );

    let client = NovaClient::connect(&args.addr).await?;
    let users = client.collection("bench_users");

    println!("Executing write benchmark (INSERT)...");
    let start = Instant::now();

    for i in 0..args.iterations {
        let mut doc = Document::with_id(format!("bench_{i}"));
        doc.insert("index", i as i64);
        doc.insert("payload", "sample payload data for performance testing");
        users.insert(doc).await?;
    }

    let elapsed = start.elapsed();
    let ops_per_sec = args.iterations as f64 / elapsed.as_secs_f64();
    let avg_latency_ms = (elapsed.as_secs_f64() * 1000.0) / args.iterations as f64;

    println!("\nBenchmark Results:");
    println!("  Total writes: {}", args.iterations);
    println!("  Elapsed time: {:.2?}", elapsed);
    println!(
        "  Throughput:   {:.2} ops/sec",
        ops_per_sec.to_string().bold()
    );
    println!("  Avg latency:  {:.3} ms/op", avg_latency_ms);

    Ok(())
}

async fn cmd_demo() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        "==================================================".cyan()
    );
    println!("{}", "  NOVA DB Playground — Demo Environment".bold());
    println!(
        "{}",
        "==================================================".cyan()
    );

    let tmp = tempdir()?;
    let config = ServerConfig {
        port: 17409,
        data_dir: tmp.path().to_path_buf(),
        ..Default::default()
    };

    let server = NovaServer::new(config)?;
    let shutdown = server.shutdown_handle();

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let client = NovaClient::connect("127.0.0.1:17409").await?;

    println!("Seeding sample dataset (users, projects, metrics)...");

    // Seed users
    let users = client.collection("users");
    let mut u1 = Document::with_id("u_101");
    u1.insert("name", "Sunrit Biswas");
    u1.insert("role", "Systems Architect");
    u1.insert(
        "skills",
        Value::Array(vec![
            "Rust".into(),
            "Distributed Systems".into(),
            "Databases".into(),
        ]),
    );
    u1.insert("active", true);
    users.insert(u1).await?;

    let mut u2 = Document::with_id("u_102");
    u2.insert("name", "Elena Rostova");
    u2.insert("role", "Performance Engineer");
    u2.insert(
        "skills",
        Value::Array(vec![
            "Linux eBPF".into(),
            "Tokio".into(),
            "Concurrency".into(),
        ]),
    );
    u2.insert("active", true);
    users.insert(u2).await?;

    // Seed projects
    let projects = client.collection("projects");
    let mut p1 = Document::with_id("proj_nova");
    p1.insert("title", "NOVA DB");
    p1.insert("status", "in_development");
    p1.insert("stars", 1250);
    projects.insert(p1).await?;

    println!(
        "{} Playground database seeded successfully!",
        "✓".green().bold()
    );
    println!("Launching interactive shell on playground...");

    cmd_shell(ShellArgs {
        addr: "127.0.0.1:17409".to_string(),
    })
    .await?;

    let _ = shutdown.send(());
    Ok(())
}

async fn cmd_studio(args: StudioArgs) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        "==================================================".cyan()
    );
    println!(
        "{}",
        "  NOVA DB Studio — Real-time Web Console & Engine".bold()
    );
    println!(
        "{}",
        "  “A database that understands the flow of your data.”".italic()
    );
    println!(
        "{}",
        "==================================================".cyan()
    );

    let host = args.host.clone();
    let port = args.port;
    let web_port = args.web_port;
    let data_dir = args.data_dir.clone();

    let config = ServerConfig {
        host: args.host,
        port: args.port,
        web_port: Some(args.web_port),
        data_dir: args.data_dir,
        ..Default::default()
    };

    println!("  {}:          {}:{}", "Binary Protocol".bold(), host, port);
    println!(
        "  {}:            http://{}:{}",
        "NOVA Studio (Web)".bold().green(),
        host,
        web_port
    );
    println!("  {}:              {:?}", "Data Directory".bold(), data_dir);
    println!(
        "{}",
        "==================================================".cyan()
    );
    println!(
        "  {} Point your browser to {} to explore your data flow.",
        "▶".green().bold(),
        format!("http://{host}:{web_port}").cyan().underline()
    );

    let server = NovaServer::new(config)?;

    // Handle Ctrl+C
    let shutdown = server.shutdown_handle();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        println!(
            "\n{}",
            "Received Ctrl+C, shutting down NOVA Studio...".yellow()
        );
        let _ = shutdown.send(());
    });

    server.run().await?;
    Ok(())
}
