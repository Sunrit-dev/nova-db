# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1] - 2026-09-23

### Added
- Embedded **NOVA Studio Web Console** with interactive NQL editor, visual Data Flow stream, and Server-Sent Events (SSE).
- Machine-readable `--json` formatting flag for `nova status`.
- High-throughput batch mutation pipeline example (`batch_pipeline`).
- Production deployment guide with kernel sysctl tuning and `docker-compose.yml`.
- NQL cheat sheet and syntax quick-reference guide.
- New unit and edge-case tests for zero-length payload NBF records and concurrent Ping/Pong frames.

## [0.1.0] - 2026-09-23

### Added
- Core typed value system (`Value`) with support for Null, Bool, Int, Float, String, Bytes, Array, Object, Timestamp, and UUID.
- Document and DataFlow event representations (`Document`, `DataEvent`, `EventType`).
- Storage engine implementing the NOVA Binary Format (NBF) with CRC32 verification.
- Write-Ahead Log (WAL) with segment rotation and durability sync modes.
- Crash recovery engine with automatic torn-write tail detection and repair.
- Point-in-time snapshot manager (`SnapshotManager`) with checksum validation.
- Extensible secondary indexing engine featuring `HashIndex` and `OrderedIndex` (B-Tree).
- NOVA Query Language (NQL) lexer, AST, Pratt expression parser, and query planner.
- NVP binary wire protocol codec for Tokio with malicious input guards.
- Concurrent async server (`NovaServer`) with connection pooling, backpressure, and graceful shutdown.
- Real-time `WATCH` query streams over Tokio broadcast channels.
- Asynchronous client library (`NovaClient`) with fluent collection handles.
- Interactive CLI and REPL (`nova`) with table rendering, diagnostics (`doctor`), storage inspector (`inspect`), and live playground (`demo`).
- Micro-benchmark harness and integration test suite covering recovery, concurrency, and protocol fuzzing.
