use crate::error::{NovaError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use uuid::Uuid;

/// Typed value representation in NOVA DB.
///
/// Designed for deterministic ordering, cross-language interoperability,
/// and safe evaluation inside the NQL query engine.
#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
    Timestamp(i64), // Microseconds since UNIX epoch
    Uuid([u8; 16]),
}

impl Value {
    /// Return the canonical type name.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::Bytes(_) => "bytes",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
            Value::Timestamp(_) => "timestamp",
            Value::Uuid(_) => "uuid",
        }
    }

    /// Check if the value is null.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Convert to boolean if applicable, or error.
    pub fn as_bool(&self) -> Result<bool> {
        match self {
            Value::Bool(b) => Ok(*b),
            other => Err(NovaError::type_error("bool", other.type_name())),
        }
    }

    /// Convert to i64 if applicable, or error.
    pub fn as_int(&self) -> Result<i64> {
        match self {
            Value::Int(i) => Ok(*i),
            Value::Float(f)
                if f.fract() == 0.0 && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 =>
            {
                Ok(*f as i64)
            }
            other => Err(NovaError::type_error("int", other.type_name())),
        }
    }

    /// Convert to f64 if applicable, or error.
    pub fn as_float(&self) -> Result<f64> {
        match self {
            Value::Float(f) => Ok(*f),
            Value::Int(i) => Ok(*i as f64),
            other => Err(NovaError::type_error("float", other.type_name())),
        }
    }

    /// Convert to string slice if applicable, or error.
    pub fn as_str(&self) -> Result<&str> {
        match self {
            Value::String(s) => Ok(s.as_str()),
            other => Err(NovaError::type_error("string", other.type_name())),
        }
    }

    /// Convert to byte slice if applicable, or error.
    pub fn as_bytes(&self) -> Result<&[u8]> {
        match self {
            Value::Bytes(b) => Ok(b.as_slice()),
            other => Err(NovaError::type_error("bytes", other.type_name())),
        }
    }

    /// Convert to array reference if applicable.
    pub fn as_array(&self) -> Result<&Vec<Value>> {
        match self {
            Value::Array(arr) => Ok(arr),
            other => Err(NovaError::type_error("array", other.type_name())),
        }
    }

    /// Convert to object reference if applicable.
    pub fn as_object(&self) -> Result<&BTreeMap<String, Value>> {
        match self {
            Value::Object(obj) => Ok(obj),
            other => Err(NovaError::type_error("object", other.type_name())),
        }
    }

    /// Returns a new timestamp with current UTC time in microseconds.
    pub fn now() -> Self {
        let now = Utc::now();
        Value::Timestamp(now.timestamp_micros())
    }

    /// Get a nested field from an Object by dot-separated path (e.g., "profile.contact.email").
    pub fn get_path(&self, path: &str) -> Option<&Value> {
        let parts: Vec<&str> = path.split('.').collect();
        let mut current = self;
        for part in parts {
            match current {
                Value::Object(map) => {
                    current = map.get(part)?;
                }
                _ => return None,
            }
        }
        Some(current)
    }

    /// Compares two numeric values (Int or Float) with promotion to Float if types differ.
    pub fn numeric_cmp(&self, other: &Value) -> Option<Ordering> {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
            (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
            (Value::Int(a), Value::Float(b)) => (*a as f64).partial_cmp(b),
            (Value::Float(a), Value::Int(b)) => a.partial_cmp(&(*b as f64)),
            _ => None,
        }
    }
}

// Equality implementation with numeric co-ercion where appropriate
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => {
                if a.is_nan() && b.is_nan() {
                    true
                } else {
                    a == b
                }
            }
            (Value::Int(a), Value::Float(b)) | (Value::Float(b), Value::Int(a)) => *a as f64 == *b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bytes(a), Value::Bytes(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => a == b,
            (Value::Object(a), Value::Object(b)) => a == b,
            (Value::Timestamp(a), Value::Timestamp(b)) => a == b,
            (Value::Uuid(a), Value::Uuid(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Value {}

// Consistent partial ordering for indexing and NQL queries
impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// Total ordering implementation for deterministic BTree indexing
impl Ord for Value {
    fn cmp(&self, other: &Self) -> Ordering {
        // Tag ordering: Null < Bool < Int/Float < String < Bytes < Timestamp < Uuid < Array < Object
        fn tag(v: &Value) -> u8 {
            match v {
                Value::Null => 0,
                Value::Bool(_) => 1,
                Value::Int(_) | Value::Float(_) => 2,
                Value::String(_) => 3,
                Value::Bytes(_) => 4,
                Value::Timestamp(_) => 5,
                Value::Uuid(_) => 6,
                Value::Array(_) => 7,
                Value::Object(_) => 8,
            }
        }

        let tag_a = tag(self);
        let tag_b = tag(other);

        if tag_a != tag_b {
            return tag_a.cmp(&tag_b);
        }

        match (self, other) {
            (Value::Null, Value::Null) => Ordering::Equal,
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            (Value::Float(a), Value::Float(b)) => a.total_cmp(b),
            (Value::Int(a), Value::Float(b)) => (*a as f64).total_cmp(b),
            (Value::Float(a), Value::Int(b)) => a.total_cmp(&(*b as f64)),
            (Value::String(a), Value::String(b)) => a.cmp(b),
            (Value::Bytes(a), Value::Bytes(b)) => a.cmp(b),
            (Value::Timestamp(a), Value::Timestamp(b)) => a.cmp(b),
            (Value::Uuid(a), Value::Uuid(b)) => a.cmp(b),
            (Value::Array(a), Value::Array(b)) => a.cmp(b),
            (Value::Object(a), Value::Object(b)) => a.cmp(b),
            _ => Ordering::Equal,
        }
    }
}

// Consistent hashing implementation that preserves PartialEq equivalence
impl std::hash::Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Value::Null => 0u8.hash(state),
            Value::Bool(b) => {
                1u8.hash(state);
                b.hash(state);
            }
            Value::Int(i) => {
                2u8.hash(state);
                i.hash(state);
            }
            Value::Float(f) => {
                2u8.hash(state);
                if f.fract() == 0.0 && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 {
                    (*f as i64).hash(state);
                } else {
                    f.to_bits().hash(state);
                }
            }
            Value::String(s) => {
                3u8.hash(state);
                s.hash(state);
            }
            Value::Bytes(b) => {
                4u8.hash(state);
                b.hash(state);
            }
            Value::Timestamp(ts) => {
                5u8.hash(state);
                ts.hash(state);
            }
            Value::Uuid(u) => {
                6u8.hash(state);
                u.hash(state);
            }
            Value::Array(arr) => {
                7u8.hash(state);
                arr.hash(state);
            }
            Value::Object(map) => {
                8u8.hash(state);
                for (k, v) in map {
                    k.hash(state);
                    v.hash(state);
                }
            }
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(fl) => write!(f, "{fl}"),
            Value::String(s) => write!(f, "\"{s}\""),
            Value::Bytes(b) => write!(f, "0x{}", hex_encode(b)),
            Value::Array(arr) => {
                write!(f, "[")?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{v}")?;
                }
                write!(f, "]")
            }
            Value::Object(map) => {
                write!(f, "{{")?;
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{k}\": {v}")?;
                }
                write!(f, "}}")
            }
            Value::Timestamp(ts) => {
                if let Some(dt) = DateTime::from_timestamp_micros(*ts) {
                    write!(f, "{}", dt.to_rfc3339())
                } else {
                    write!(f, "timestamp({ts})")
                }
            }
            Value::Uuid(u) => {
                let id = Uuid::from_bytes(*u);
                write!(f, "uuid(\"{id}\")")
            }
        }
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut s, "{:02x}", b);
    }
    s
}

// Serde implementation
impl Serialize for Value {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serde_json::Value::from(self.clone()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let json_val = serde_json::Value::deserialize(deserializer)?;
        Ok(Value::from(json_val))
    }
}

// Conversions from serde_json::Value
impl From<serde_json::Value> for Value {
    fn from(json: serde_json::Value) -> Self {
        match json {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(b),
            serde_json::Value::Number(num) => {
                if let Some(i) = num.as_i64() {
                    Value::Int(i)
                } else if let Some(f) = num.as_f64() {
                    Value::Float(f)
                } else {
                    Value::Null
                }
            }
            serde_json::Value::String(s) => {
                // If it parses cleanly as UUID, represent as Uuid
                if let Ok(u) = Uuid::parse_str(&s) {
                    Value::Uuid(*u.as_bytes())
                } else {
                    Value::String(s)
                }
            }
            serde_json::Value::Array(arr) => {
                Value::Array(arr.into_iter().map(Value::from).collect())
            }
            serde_json::Value::Object(obj) => {
                let mut map = BTreeMap::new();
                for (k, v) in obj {
                    map.insert(k, Value::from(v));
                }
                Value::Object(map)
            }
        }
    }
}

impl From<Value> for serde_json::Value {
    fn from(val: Value) -> Self {
        match val {
            Value::Null => serde_json::Value::Null,
            Value::Bool(b) => serde_json::Value::Bool(b),
            Value::Int(i) => serde_json::Value::Number(i.into()),
            Value::Float(f) => serde_json::Number::from_f64(f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Value::String(s) => serde_json::Value::String(s),
            Value::Bytes(b) => serde_json::Value::String(format!("0x{}", hex_encode(&b))),
            Value::Array(arr) => {
                serde_json::Value::Array(arr.into_iter().map(serde_json::Value::from).collect())
            }
            Value::Object(map) => {
                let mut obj = serde_json::Map::new();
                for (k, v) in map {
                    obj.insert(k, serde_json::Value::from(v));
                }
                serde_json::Value::Object(obj)
            }
            Value::Timestamp(ts) => serde_json::Value::Number(ts.into()),
            Value::Uuid(u) => serde_json::Value::String(Uuid::from_bytes(u).to_string()),
        }
    }
}

// Convenient From implementations for native Rust types
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Int(v as i64)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_string())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}

impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Self {
        Value::Bytes(v)
    }
}

impl From<Uuid> for Value {
    fn from(u: Uuid) -> Self {
        Value::Uuid(*u.as_bytes())
    }
}
