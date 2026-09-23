# NOVA Protocol (NVP) Specification

## 1. Protocol Architecture

The NOVA Protocol (NVP) is a binary, framed, multiplexed application-layer protocol designed to run over TCP (default port: `7400`).

### 1.1 Wire Frame Structure

```
+------------+-------------+------------+-------+----------------+----------------+-----------------+----------+
| MAGIC      | VERSION     | FRAME_TYPE | FLAGS | REQUEST_ID     | PAYLOAD_LENGTH | PAYLOAD         | CHECKSUM |
| 4 Bytes    | 2 Bytes     | 1 Byte     | 1 B   | 8 Bytes        | 4 Bytes        | N Bytes         | 4 Bytes  |
+------------+-------------+------------+-------+----------------+----------------+-----------------+----------+
```

- **MAGIC**: `[0x4E, 0x56, 0x50, 0x31]` (`"NVP1"`).
- **VERSION**: `u16` Big Endian (`1`).
- **FRAME_TYPE**: `u8` indicating frame category:
  - `0x01`: `Request` (Query / Command from client to server)
  - `0x02`: `Response` (Result payload from server to client)
  - `0x03`: `Event` (Live `DataEvent` pushed to WATCH subscribers)
  - `0x04`: `Error` (Structured error notification)
  - `0x05`: `Ping` (Heartbeat check)
  - `0x06`: `Pong` (Heartbeat response)
- **FLAGS**: `u8` reserved for compression and encryption flags.
- **REQUEST_ID**: `u64` Big Endian identifier used to multiplex concurrent requests over a single TCP stream.
- **PAYLOAD_LENGTH**: `u32` Big Endian length of the payload in bytes (Max: 16 MB).
- **PAYLOAD**: Serialized JSON or binary payload.
- **CHECKSUM**: `u32` IEEE CRC32 over Header + Payload.

---

## 2. Request and Response Lifecycle

### 2.1 Standard Query Execution
1. Client generates monotonic `request_id`.
2. Encodes `RequestPayload::Query { nql }` into an NVP frame and writes to TCP socket.
3. Server receives and decodes frame, computes CRC32, parses NQL, executes against storage engine.
4. Server replies with `NvpFrame` with type `Response` using matching `request_id`.

### 2.2 WATCH Real-Time Subscriptions
1. Client sends `RequestPayload::Watch { nql: "WATCH users WHERE age > 21" }` with `request_id = 42`.
2. Server registers subscription and returns `ResponsePayload::WatchAck { subscription_id: 42 }`.
3. Whenever mutations occur that match the predicate, server pushes frames with `frame_type = Event` and `request_id = 42`.
4. The client dispatches events directly to the corresponding `EventStream`.
