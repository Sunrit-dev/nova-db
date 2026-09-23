use nova_core::document::Document;
use nova_core::value::Value;
use nova_index::IndexType;

/// Complete AST for NQL (NOVA Query Language).
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Find {
        collection: String,
        filter: Option<Expr>,
        sort: Option<SortClause>,
        limit: Option<usize>,
        offset: Option<usize>,
    },
    Insert {
        collection: String,
        document: Document,
    },
    Update {
        collection: String,
        assignments: Vec<Assignment>,
        filter: Option<Expr>,
    },
    Remove {
        collection: String,
        filter: Option<Expr>,
    },
    Watch {
        collection: String,
        filter: Option<Expr>,
    },
    Count {
        collection: String,
        filter: Option<Expr>,
    },
    Exists {
        collection: String,
        filter: Option<Expr>,
    },
    CreateIndex {
        collection: String,
        field: String,
        index_type: IndexType,
    },
    DropIndex {
        collection: String,
        field: String,
    },
    Begin,
    Commit,
    Rollback,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Value),
    Field(String),
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    In {
        expr: Box<Expr>,
        list: Vec<Expr>,
    },
    Between {
        expr: Box<Expr>,
        min: Box<Expr>,
        max: Box<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Eq,
    NotEq,
    Gt,
    GtEq,
    Lt,
    LtEq,
    And,
    Or,
    Plus,
    Minus,
    Multiply,
    Divide,
    Modulo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Negate,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    pub field: String,
    pub expr: Expr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SortClause {
    pub field: String,
    pub direction: SortDirection,
}
