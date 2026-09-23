# NOVA DB — Architecture & Internals

## 1. Overview

NOVA DB is an asynchronous document-oriented database written in Rust. It was designed from the ground up around the concept of **Data Flows**: every mutation is structured as an immutable, typed `DataEvent` that flows through storage, indexing, and real-time subscription pipelines.

```
DATABASE ──► NAMESPACE ──► COLLECTION ──► DOCUMENT ──► FIELD ──► EVENT ──► INDEX ──► QUERY
```

---

## 2. Core Subsystems

### 2.1 Storage & WAL Subsystem (`nova-storage`)
- **Write-Ahead Log (WAL)**: All mutations are sequentially written to an append-only commit log before in-memory state is updated.
- **NOVA Binary Format (NBF)**: A custom binary wire and storage format featuring explicit framing, versioning, and IEEE 802.3 CRC32 checksums.
- **Segmentation**: WAL files are segmented (e.g. `0000000000000000.wal`) with automatic rolling to avoid monolithic log growth.
- **Crash Recovery**: Reads and validates segment records sequentially on startup. Safely detects and truncates partial or torn writes at the tail without losing committed transactions.

### 2.2 Indexing Subsystem (`nova-index`)
- Abstracted behind the `Index` trait:
  ```rust
  pub trait Index: Send + Sync {
      fn name(&self) -> &str;
      fn field(&self) -> &str;
      fn index_type(&self) -> IndexType;
      fn insert(&mut self, key: &Value, doc_id: &DocumentId) -> Result<(), IndexError>;
      fn remove(&mut self, key: &Value, doc_id: &DocumentId) -> Result<(), IndexError>;
      fn lookup(&self, key: &Value) -> Vec<DocumentId>;
      fn scan(&self, range: &IndexRange) -> Result<Vec<DocumentId>, IndexError>;
      fn len(&self) -> usize;
      fn clear(&mut self);
  }
  ```
- **HashIndex**: In-memory hash index offering $O(1)$ exact-match lookups.
- **OrderedIndex**: In-memory B-Tree index offering $O(\log N)$ point lookups and range scans.
- **IndexManager**: Coordinates index lifecycle and automatically updates registered indices on `INSERT`, `UPDATE`, and `REMOVE`.

### 2.3 Query & Parser Subsystem (`nova-query`)
- **Lexer**: Tokenizes input strings while tracking source lines and columns for detailed error diagnostics.
- **Pratt Parser**: Evaluates operator precedence cleanly without recursion overflow or brittle string splitting.
- **Query Planner**: Inspects expressions to choose between an `IndexLookup`, `IndexRangeScan`, or `SeqScan`.

### 2.4 Protocol & Networking Subsystem (`nova-protocol`, `nova-server`)
- **NVP Wire Protocol**: Framed binary network protocol over TCP.
- **Framing**: `[4B Magic][2B Version][1B Type][1B Flags][8B ReqID][4B PayloadLen][Payload...][4B CRC32]`.
- **Tokio Concurrency**: Non-blocking I/O with connection limit semaphores and graceful shutdown hooks.
- **Event Streaming**: Uses a broadcast channel ring buffer with lag detection to stream real-time events to `WATCH` subscribers.
