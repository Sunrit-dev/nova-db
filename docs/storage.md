# NOVA DB — Storage & NBF Specification

## 1. NOVA Binary Format (NBF)

The NOVA Binary Format (NBF) is the fundamental binary representation for write-ahead log records and persistent storage segments in NOVA DB.

### 1.1 Binary Frame Layout

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       MAGIC ("NOVB")                          |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|        VERSION (1)            |  RECORD_TYPE  |     FLAGS     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
+                       SEQUENCE (8 Bytes)                      +
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
+                   TRANSACTION ID (8 Bytes)                    +
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
+                    TIMESTAMP (8 Bytes, µs)                    +
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|        NS_LEN (2 Bytes)       |       NAMESPACE (UTF-8) ...   |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|       COLL_LEN (2 Bytes)      |      COLLECTION (UTF-8) ...   |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|      DOC_ID_LEN (2 Bytes)     |     DOCUMENT_ID (UTF-8) ...   |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     PAYLOAD_LEN (4 Bytes)                     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
+                     PAYLOAD BYTES (N Bytes)                   +
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                   CHECKSUM (4 Bytes, CRC32)                   |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

### 1.2 Record Types

| Hex | Record Type | Description |
|---|---|---|
| `0x01` | `Insert` | New document created |
| `0x02` | `Update` | Existing document updated |
| `0x03` | `Delete` | Document tombstone / removal |
| `0x04` | `Checkpoint`| Consistency checkpoint |
| `0x05` | `TxBegin` | Atomic transaction start |
| `0x06` | `TxCommit` | Atomic transaction commit |
| `0x07` | `TxRollback`| Atomic transaction rollback |

---

## 2. Checksum Computation

The 32-bit CRC32 checksum is computed over the entire frame starting from the magic bytes up to the final byte of the payload using the standard IEEE 802.3 polynomial:

$$\text{Checksum} = \text{CRC32}(\text{Magic} \mathbin{\Vert} \text{Header} \mathbin{\Vert} \text{Strings} \mathbin{\Vert} \text{Payload})$$

If the decoded checksum does not match the recomputed checksum during recovery or reading, the record is immediately flagged as corrupted.
