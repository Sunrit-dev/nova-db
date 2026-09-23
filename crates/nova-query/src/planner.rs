use crate::ast::{BinaryOp, Expr, SortClause};
use nova_core::value::Value;
use nova_index::{IndexManager, IndexRange, IndexType};

/// Execution plan chosen by the query optimizer.
#[derive(Debug, Clone, PartialEq)]
pub enum QueryPlan {
    /// Exact match via secondary index lookup
    IndexLookup {
        index_name: String,
        key: Value,
        filter: Option<Expr>,
        sort: Option<SortClause>,
        limit: Option<usize>,
        offset: Option<usize>,
    },
    /// Range scan via ordered index
    IndexRangeScan {
        index_name: String,
        range: IndexRange,
        filter: Option<Expr>,
        sort: Option<SortClause>,
        limit: Option<usize>,
        offset: Option<usize>,
    },
    /// Full sequential scan over all collection documents
    SeqScan {
        filter: Option<Expr>,
        sort: Option<SortClause>,
        limit: Option<usize>,
        offset: Option<usize>,
    },
}

pub struct QueryPlanner;

impl QueryPlanner {
    /// Plan a FIND or COUNT query using available indices.
    pub fn plan(
        filter: Option<Expr>,
        sort: Option<SortClause>,
        limit: Option<usize>,
        offset: Option<usize>,
        index_mgr: &IndexManager,
    ) -> QueryPlan {
        if let Some(ref expr) = filter {
            // Check for exact match: field == literal
            if let Expr::Binary {
                op: BinaryOp::Eq,
                left,
                right,
            } = expr
            {
                if let (Expr::Field(field), Expr::Literal(lit)) = (left.as_ref(), right.as_ref()) {
                    if let Some(idx) = index_mgr.find_index_for_field(field) {
                        return QueryPlan::IndexLookup {
                            index_name: idx.name().to_string(),
                            key: lit.clone(),
                            filter: None,
                            sort,
                            limit,
                            offset,
                        };
                    }
                } else if let (Expr::Literal(lit), Expr::Field(field)) =
                    (left.as_ref(), right.as_ref())
                {
                    if let Some(idx) = index_mgr.find_index_for_field(field) {
                        return QueryPlan::IndexLookup {
                            index_name: idx.name().to_string(),
                            key: lit.clone(),
                            filter: None,
                            sort,
                            limit,
                            offset,
                        };
                    }
                }
            }

            // Check for range match: field > lit, field >= lit, field < lit, field <= lit
            if let Expr::Binary { op, left, right } = expr {
                if let (Expr::Field(field), Expr::Literal(lit)) = (left.as_ref(), right.as_ref()) {
                    if let Some(idx) = index_mgr.find_index_for_field(field) {
                        if idx.index_type() == IndexType::Ordered {
                            let range = match op {
                                BinaryOp::Gt => IndexRange {
                                    start: std::ops::Bound::Excluded(lit.clone()),
                                    end: std::ops::Bound::Unbounded,
                                },
                                BinaryOp::GtEq => IndexRange::from_inclusive(lit.clone()),
                                BinaryOp::Lt => IndexRange {
                                    start: std::ops::Bound::Unbounded,
                                    end: std::ops::Bound::Excluded(lit.clone()),
                                },
                                BinaryOp::LtEq => IndexRange::to_inclusive(lit.clone()),
                                _ => IndexRange::all(),
                            };

                            if range != IndexRange::all() {
                                return QueryPlan::IndexRangeScan {
                                    index_name: idx.name().to_string(),
                                    range,
                                    filter: None,
                                    sort,
                                    limit,
                                    offset,
                                };
                            }
                        }
                    }
                }
            }

            // Check for BETWEEN: field BETWEEN min AND max
            if let Expr::Between {
                expr: inner,
                min,
                max,
            } = expr
            {
                if let (Expr::Field(field), Expr::Literal(min_val), Expr::Literal(max_val)) =
                    (inner.as_ref(), min.as_ref(), max.as_ref())
                {
                    if let Some(idx) = index_mgr.find_index_for_field(field) {
                        if idx.index_type() == IndexType::Ordered {
                            return QueryPlan::IndexRangeScan {
                                index_name: idx.name().to_string(),
                                range: IndexRange::between_inclusive(
                                    min_val.clone(),
                                    max_val.clone(),
                                ),
                                filter: None,
                                sort,
                                limit,
                                offset,
                            };
                        }
                    }
                }
            }
        }

        QueryPlan::SeqScan {
            filter,
            sort,
            limit,
            offset,
        }
    }
}
