# NOVA DB — NQL Cheat Sheet & Developer Reference

This quick reference covers the core syntax, operators, and functions available in the **Nova Query Language (NQL)**.

---

## 1. Document Retrieval (`FIND`)

```sql
-- Retrieve all documents in a collection
FIND users;

-- Filter with equality and logical operators
FIND users WHERE age >= 21 AND role == "engineer";

-- Range scan with string prefixes and ordering
FIND sensors WHERE temp BETWEEN 50.0 AND 85.0 SORT temp DESC;

-- Paginated queries
FIND telemetry WHERE status == "nominal" SORT timestamp DESC LIMIT 20 SKIP 40;
```

---

## 2. Document Mutation (`INSERT`, `UPDATE`, `REMOVE`)

```sql
-- Insert a single JSON-compatible document
INSERT INTO users {
  id: "usr_101",
  name: "Elena Rostova",
  role: "Lead Architect",
  points: 98,
  skills: ["Rust", "Tokio", "Storage Engines"]
};

-- In-place document updates
UPDATE users SET role = "Principal Architect", points = points + 5 WHERE id == "usr_101";

-- Conditional removal
REMOVE users WHERE points < 50;
```

---

## 3. Real-Time Change Streams (`WATCH`)

```sql
-- Stream live mutations matching a filter predicate
WATCH sensors WHERE temp > 80.0;

-- Stream all events in a collection
WATCH telemetry;
```

---

## 4. Index DDL (`CREATE INDEX`, `DROP INDEX`)

```sql
-- Create an equality Hash Index ($O(1)$)
CREATE INDEX ON users (email);

-- Create an ordered B-Tree Index for range queries
CREATE INDEX ON sensors (temp);

-- Drop an existing index
DROP INDEX ON users (email);
```

---

## 5. Aggregations & Existence

```sql
-- Count matching documents
COUNT users WHERE points >= 80;

-- Fast existence check
EXISTS sensors WHERE status == "critical";
```
