# NOVA DB — Recovery Model & Durability

## 1. Durability Guarantees

NOVA DB guarantees that any mutation acknowledged as successful under `sync_mode = always` is durably flushed to non-volatile storage (`fsync`) before the acknowledgment is transmitted to the client.

---

## 2. Recovery Process on Startup

1. **Discovery**: Scans the configured `wal/` directory and identifies all `.wal` segment files sorted in ascending sequence order.
2. **Sequential Validation**: Opens each segment file and validates each record:
   - Verifies NBF Magic (`NOVB`).
   - Verifies Storage Version (`1`).
   - Verifies record length limits.
   - Computes IEEE 802.3 CRC32 checksum over the record and asserts equality with the recorded checksum.
3. **Torn-Write Detection & Repair**: If a server or host system loses power in the middle of a disk write, the final bytes at the log tail may be partial. NOVA DB:
   - Identifies the incomplete trailing bytes at the end of the segment.
   - Truncates the segment file cleanly back to the last validated byte offset.
   - Logs an audit message detailing the torn write truncation.
4. **Replay & State Reconstruction**:
   - Replays all validated `Insert`, `Update`, and `Delete` records into in-memory collections.
   - Aligns the WAL sequence number to the highest sequence observed in the log.
   - Reconstructs all secondary indexes from the recovered document collections.
5. **Ready State**: Only after recovery has completed does the network listener begin accepting client connections.
