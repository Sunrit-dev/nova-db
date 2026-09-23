# NOVA DB

> **“A database that understands the flow of your data.”**

NOVA is an asynchronous, event-driven document database written from scratch in Rust. It combines durable append-only storage, typed documents, indexed queries, and real-time change streams behind a custom binary network protocol.

---

## Why NOVA Exists

Most modern applications do not simply query static rows—they process continuous streams of data mutations. Traditional relational databases and key-value stores treat event streaming as an external concern, forcing developers to manage complex external CDC (Change Data Capture) pipelines, Kafka connectors, or Redis Pub/Sub channels alongside their primary database.

NOVA DB was designed from the ground up to unify storage and event streams using a single underlying concept: **Data Flows**.

```
DATABASE ───► NAMESPACE ───► COLLECTION ───► DOCUMENT ───► FIELD ───► EVENT ───► INDEX ───► QUERY
```

Every state change—whether an `INSERT`, `UPDATE`, `REMOVE`, or `EXPIRE`—is deterministically captured as a first-class `DataEvent`. These events are simultaneously:
1. Sequentially committed to a checksummed append-only **Write-Ahead Log (WAL)** in the custom **NOVA Binary Format (NBF)**.
2. Updated in memory and reflected across secondary hash and B-Tree indexes.
3. Broadcast over backpressure-guarded asynchronous network streams directly to clients issuing `WATCH` queries.

---

## Key Characteristics

- **Zero-Dependency Core Storage**: Implements its own binary storage format (**NBF**), segment manager, and deterministic crash recovery engine with torn-write tail repair and CRC32 verification.
- **NOVA Query Language (NQL)**: A purpose-built, human-readable query language featuring an extensible Pratt expression parser, index-aware query planner, and first-class `WATCH` streams.
- **NVP Wire Protocol**: A versioned, framed binary network protocol with CRC32 integrity validation, multiplexed request/response channels, and streaming event delivery.
- **Built on Tokio**: Asynchronous I/O, non-blocking connection dispatch, fine-grained semaphore concurrency limits, and clean cancellation.
- **Self-Contained Developer Tooling**: Includes an interactive shell (`nova shell`) with syntax rendering, storage inspection (`nova inspect`), backup/snapshot restoration (`nova backup`), system diagnostics (`nova doctor`), and an in-memory playground (`nova demo`).

---

## System Architecture

```
                                  Client Connection (TCP)
                                             │
                                             ▼
                                     ┌───────────────┐
                                     │   NVP Codec   │ (Framing, CRC32, Limits)
                                     └───────┬───────┘
                                             │
                        ┌────────────────────┴────────────────────┐
                        ▼                                         ▼
               ┌─────────────────┐                       ┌─────────────────┐
               │  NQL Parser     │                       │  Event Stream   │
               │  & Evaluator    │                       │  Broker (WATCH) │
               └────────┬────────┘                       └────────▲────────┘
                        │                                         │
                        ▼                                         │
               ┌─────────────────┐                                │
               │  Query Planner  │                                │
               └────────┬────────┘                                │
                        │                                         │
       ┌────────────────┴────────────────┐                        │
       ▼                                 ▼                        │
┌──────────────┐                  ┌──────────────┐                │
│ Index Engine │                  │ Memory Store │────────────► (Publishes
│ Hash / BTree │                  │  Collections │               DataEvents)
└──────────────┘                  └──────┬───────┘
                                         │
                                         ▼
                              ┌──────────────────────┐
                              │  WAL & Recovery Mgr  │
                              │ (NBF Segments + CRC) │
                              └──────────────────────┘
```

---

## Quick Start

### 1. Build the Workspace

```bash
git clone https://github.com/nova-db/nova.git
cd nova
cargo build --release
```

### 2. Launch the Database Server

```bash
cargo run --bin nova -- start --port 7400 --data-dir ./data
```

### 3. Launch the Interactive Shell

In a separate terminal:

```bash
cargo run --bin nova -- shell
```

Or instantly try the self-contained sandbox with pre-seeded data:

```bash
cargo run --bin nova -- demo
```

---

## NQL — NOVA Query Language Reference

### Inserting Documents

```sql
INSERT INTO users VALUES ({
  "name": "Sunrit Biswas",
  "role": "Systems Architect",
  "age": 28,
  "skills": ["Rust", "Distributed Systems", "Networking"],
  "active": true
})
```

### Querying Documents

```sql
-- Point queries and filtered range scans
FIND users WHERE age >= 21 AND active == true SORT age DESC LIMIT 10

-- Nested path queries
FIND users WHERE profile.role == "engineer"

-- Membership and range tests
FIND users WHERE age BETWEEN 20 AND 35
FIND users WHERE country IN ("IN", "US", "DE")
```

### Real-Time Change Streams (WATCH)

Clients can subscribe to live mutations filtered at the database level:

```sql
WATCH users WHERE age > 25
```

When mutations occur, the server streams live events:

```
[18:45:10.123] CREATE nova://default/users/u_101 => {"name": "Sunrit Biswas", "role": "Systems Architect"}
[18:45:12.450] UPDATE nova://default/users/u_101 => {"role": "Lead Architect"}
```

### Updating & Removing

```sql
UPDATE users SET role = "Lead Architect", age = age + 1 WHERE id == "u_101"
REMOVE users WHERE active == false
```

### Index Management

```sql
-- Exact-match Hash Index
CREATE INDEX users.email TYPE hash

-- Range-scan B-Tree Index
CREATE INDEX users.created_at TYPE ordered

-- Drop Index
DROP INDEX users.email
```

---

## Official Rust Client (`nova-client`)

Add `nova-client` to your `Cargo.toml`:

```rust
use nova_client::NovaClient;
use nova_core::doc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = NovaClient::connect("127.0.0.1:7400").await?;
    let users = client.collection("users");

    // 1. Subscribe to live change events
    let mut stream = users.watch(Some("age > 21")).await?;

    tokio::spawn(async move {
        while let Some(event) = stream.next().await {
            println!("Received DataEvent: {:?}", event.target_uri());
        }
    });

    // 2. Insert document
    let user = users.insert(doc! {
        "name" => "Alice",
        "role" => "Developer",
        "age" => 29,
    }).await?;

    // 3. Query documents
    let results = users.find("age >= 25").await?;
    println!("Found {} users", results.len());

    Ok(())
}
```

---

## CLI Tooling Overview

| Command | Description |
|---|---|
| `nova start` | Launches the asynchronous TCP database server and Web Studio |
| `nova studio` | Starts the database engine and serves the embedded NOVA Studio Web Console |
| `nova status` | Pings the server and inspects live connection & throughput metrics |
| `nova shell` | Starts the interactive NQL REPL with history and formatted tables |
| `nova demo` | Launches an ephemeral sandbox with preloaded datasets |
| `nova inspect <file>` | Parses and validates checksums on WAL segment or snapshot files |
| `nova doctor` | Runs health checks on the storage directory and log segments |
| `nova backup create` | Captures a point-in-time snapshot with CRC32 validation |
| `nova backup restore` | Restores database state from a validated snapshot file |
| `nova benchmark` | Runs throughput and latency percentile benchmarks |

---

## Storage Engine & Recovery

NOVA uses an append-only commit log segmented into rolling files:

```
data/
├── wal/
│   ├── 0000000000000000.wal
│   └── 0000000000000001.wal
└── snapshots/
    ├── snap_20260923.snap
    └── snap_20260923.meta.json
```

### Crash Recovery Invariant

On startup, NOVA DB scans all WAL segments in sequence order:
1. Validates magic bytes (`NOVB`), version, record lengths, and IEEE CRC32 checksums.
2. If an incomplete record is detected at the log tail (simulating power failure during write), the engine **safely truncates the segment to the last valid byte offset** and resumes cleanly without data loss of previously acknowledged records.
3. Valid mutations are deterministically replayed to reconstruct in-memory state and secondary indices.

---

## Crates in Workspace

| Crate | Responsibility |
|---|---|
| `nova-core` | Typed data model (`Value`, `Document`, `DataEvent`), error definitions |
| `nova-storage` | NBF binary format, WAL segment rotation, crash recovery, snapshots |
| `nova-index` | Extensible index trait, HashIndex, OrderedIndex (B-Tree), IndexManager |
| `nova-query` | Handcrafted NQL lexer, AST, Pratt expression parser, query planner |
| `nova-protocol` | NVP binary wire protocol, frame layout, checksums, Tokio codec |
| `nova-server` | Concurrent Tokio TCP server, connection tasks, WATCH event broker |
| `nova-client` | Asynchronous multiplexed client library with streaming subscriptions |
| `nova-cli` | Developer CLI, interactive NQL shell, storage inspector, doctor |
| `nova-testkit` | Fault injection, crash simulations, and concurrency test harness |

---

## Verification & Testing

Run the complete test suite:

```bash
# Run all unit and integration tests
cargo test --workspace

# Run static analysis and clippy checks
cargo clippy --all-targets -- -D warnings

# Verify formatting
cargo fmt --check
```

---

## Documentation

- [Architecture & Internal Design](docs/architecture.md)
- [Storage Format & WAL](docs/storage.md)
- [NVP Network Protocol](docs/protocol.md)
- [NQL Query Language](docs/query-language.md)
- [Crash Recovery & Durability](docs/recovery.md)
- [Contributing Guide](docs/contributing.md)

---

## License

Dual-licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE) or http://opensource.org/licenses/MIT)

at your option.
