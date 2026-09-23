use crate::ast::{Assignment, BinaryOp, Expr, UnaryOp};
use nova_core::document::Document;
use nova_core::error::{NovaError, Result};
use nova_core::value::Value;
use std::cmp::Ordering;

/// Evaluates NQL expressions against documents.
pub struct Evaluator;

impl Evaluator {
    /// Evaluate expression into a typed Value.
    pub fn eval(expr: &Expr, doc: &Document) -> Result<Value> {
        match expr {
            Expr::Literal(val) => Ok(val.clone()),
            Expr::Field(path) => {
                if path == "id" || path == "_id" {
                    Ok(Value::String(doc.id.as_str().to_string()))
                } else if let Some(val) = doc.get_path(path).or_else(|| doc.fields.get(path)) {
                    Ok(val.clone())
                } else {
                    Ok(Value::Null)
                }
            }
            Expr::Unary { op, expr } => {
                let inner = Self::eval(expr, doc)?;
                match op {
                    UnaryOp::Not => {
                        let b = Self::is_truthy(&inner);
                        Ok(Value::Bool(!b))
                    }
                    UnaryOp::Negate => match inner {
                        Value::Int(i) => Ok(Value::Int(-i)),
                        Value::Float(f) => Ok(Value::Float(-f)),
                        other => Err(NovaError::type_error("numeric", other.type_name())),
                    },
                }
            }
            Expr::Binary { op, left, right } => {
                // Short-circuit logical operations
                if *op == BinaryOp::And {
                    let l_val = Self::eval(left, doc)?;
                    if !Self::is_truthy(&l_val) {
                        return Ok(Value::Bool(false));
                    }
                    let r_val = Self::eval(right, doc)?;
                    return Ok(Value::Bool(Self::is_truthy(&r_val)));
                }
                if *op == BinaryOp::Or {
                    let l_val = Self::eval(left, doc)?;
                    if Self::is_truthy(&l_val) {
                        return Ok(Value::Bool(true));
                    }
                    let r_val = Self::eval(right, doc)?;
                    return Ok(Value::Bool(Self::is_truthy(&r_val)));
                }

                let l_val = Self::eval(left, doc)?;
                let r_val = Self::eval(right, doc)?;

                match op {
                    BinaryOp::Eq => Ok(Value::Bool(l_val == r_val)),
                    BinaryOp::NotEq => Ok(Value::Bool(l_val != r_val)),
                    BinaryOp::Gt => Ok(Value::Bool(l_val.cmp(&r_val) == Ordering::Greater)),
                    BinaryOp::GtEq => {
                        let ord = l_val.cmp(&r_val);
                        Ok(Value::Bool(
                            ord == Ordering::Greater || ord == Ordering::Equal,
                        ))
                    }
                    BinaryOp::Lt => Ok(Value::Bool(l_val.cmp(&r_val) == Ordering::Less)),
                    BinaryOp::LtEq => {
                        let ord = l_val.cmp(&r_val);
                        Ok(Value::Bool(ord == Ordering::Less || ord == Ordering::Equal))
                    }
                    BinaryOp::Plus => match (&l_val, &r_val) {
                        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
                        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
                        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + b)),
                        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + *b as f64)),
                        (Value::String(a), Value::String(b)) => {
                            Ok(Value::String(format!("{a}{b}")))
                        }
                        _ => Err(NovaError::invalid_query(
                            "Cannot perform '+' on given types",
                        )),
                    },
                    BinaryOp::Minus => match (&l_val, &r_val) {
                        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a - b)),
                        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
                        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 - b)),
                        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a - *b as f64)),
                        _ => Err(NovaError::invalid_query(
                            "Cannot perform '-' on given types",
                        )),
                    },
                    BinaryOp::Multiply => match (&l_val, &r_val) {
                        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a * b)),
                        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
                        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 * b)),
                        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a * *b as f64)),
                        _ => Err(NovaError::invalid_query(
                            "Cannot perform '*' on given types",
                        )),
                    },
                    BinaryOp::Divide => match (&l_val, &r_val) {
                        (Value::Int(a), Value::Int(b)) => {
                            if *b == 0 {
                                return Err(NovaError::invalid_query("Division by zero"));
                            }
                            Ok(Value::Int(a / b))
                        }
                        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
                        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 / b)),
                        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a / *b as f64)),
                        _ => Err(NovaError::invalid_query(
                            "Cannot perform '/' on given types",
                        )),
                    },
                    BinaryOp::Modulo => match (&l_val, &r_val) {
                        (Value::Int(a), Value::Int(b)) => {
                            if *b == 0 {
                                return Err(NovaError::invalid_query("Modulo by zero"));
                            }
                            Ok(Value::Int(a % b))
                        }
                        _ => Err(NovaError::invalid_query(
                            "Modulo '%' only supported for integers",
                        )),
                    },
                    BinaryOp::And | BinaryOp::Or => unreachable!(),
                }
            }
            Expr::Between { expr, min, max } => {
                let val = Self::eval(expr, doc)?;
                let min_val = Self::eval(min, doc)?;
                let max_val = Self::eval(max, doc)?;
                let ok =
                    val.cmp(&min_val) != Ordering::Less && val.cmp(&max_val) != Ordering::Greater;
                Ok(Value::Bool(ok))
            }
            Expr::In { expr, list } => {
                let val = Self::eval(expr, doc)?;
                for item in list {
                    let item_val = Self::eval(item, doc)?;
                    if val == item_val {
                        return Ok(Value::Bool(true));
                    }
                }
                Ok(Value::Bool(false))
            }
        }
    }

    /// Check if expression evaluates to boolean true for filtering.
    pub fn matches_filter(filter: &Option<Expr>, doc: &Document) -> Result<bool> {
        match filter {
            Some(expr) => {
                let val = Self::eval(expr, doc)?;
                Ok(Self::is_truthy(&val))
            }
            None => Ok(true),
        }
    }

    /// Apply UPDATE assignments to a document.
    pub fn apply_assignments(doc: &mut Document, assignments: &[Assignment]) -> Result<()> {
        for assign in assignments {
            let val = Self::eval(&assign.expr, doc)?;
            if assign.field == "id" || assign.field == "_id" {
                return Err(NovaError::invalid_query("Cannot modify primary key 'id'"));
            }
            doc.insert(&assign.field, val);
        }
        doc.advance_version();
        Ok(())
    }

    fn is_truthy(val: &Value) -> bool {
        match val {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0 && !f.is_nan(),
            Value::String(s) => !s.is_empty(),
            Value::Bytes(b) => !b.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
            Value::Timestamp(t) => *t != 0,
            Value::Uuid(_) => true,
        }
    }
}
