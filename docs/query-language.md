# NOVA Query Language (NQL) Reference

NQL is an intuitive, expressive query language designed specifically for document data flows and live streams.

---

## 1. FIND Queries

```sql
FIND <collection> [WHERE <expr>] [SORT <field> [ASC|DESC]] [LIMIT <n>] [OFFSET <n>]
```

### Examples
```sql
FIND users WHERE age > 18
FIND users WHERE country == "IN" SORT created_at DESC LIMIT 25
FIND orders WHERE total >= 100.0 AND status == "completed"
FIND users WHERE profile.contact.verified == true
```

---

## 2. INSERT Queries

```sql
INSERT INTO <collection> VALUES ({ <fields> })
```

### Example
```sql
INSERT INTO users VALUES ({
  "name": "Sunrit",
  "role": "Systems Engineer",
  "age": 28,
  "skills": ["Rust", "Python", "Linux"]
})
```

---

## 3. UPDATE Queries

```sql
UPDATE <collection> SET <field> = <expr> (, <field> = <expr>)* [WHERE <expr>]
```

### Example
```sql
UPDATE users SET role = "Lead Engineer", age = age + 1 WHERE id == "u_101"
```

---

## 4. REMOVE Queries

```sql
REMOVE <collection> [WHERE <expr>]
```

### Example
```sql
REMOVE users WHERE inactive == true
```

---

## 5. WATCH Queries

Subscribes to live mutations in real time.

```sql
WATCH <collection> [WHERE <expr>]
```

### Example
```sql
WATCH telemetry WHERE temperature > 80.0
```

---

## 6. Index DDL

```sql
CREATE INDEX <collection>.<field> [TYPE hash|ordered]
DROP INDEX <collection>.<field>
```

### Example
```sql
CREATE INDEX users.email TYPE hash
CREATE INDEX users.age TYPE ordered
DROP INDEX users.email
```
