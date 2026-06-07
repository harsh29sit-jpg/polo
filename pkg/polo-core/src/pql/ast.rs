use chrono::{DateTime, Utc};

use crate::clock::Hlc;

#[derive(Debug, Clone)]
pub struct Query {
    pub columns: Vec<Column>,
    pub namespace: String,
    pub filter: Option<Expr>,
    pub effective_at: Option<DateTime<Utc>>,
    pub asof: Option<Hlc>,
    pub order_by: Vec<OrderBy>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone)]
pub enum Column {
    Star,
    Named(String),
}

#[derive(Debug, Clone)]
pub struct OrderBy {
    pub column: String,
    pub direction: Direction,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Direction {
    Asc,
    Desc,
}

#[derive(Debug, Clone)]
pub enum Expr {
    BinOp {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    IsNull(Box<Expr>),
    IsNotNull(Box<Expr>),
    Like {
        expr: Box<Expr>,
        pattern: String,
    },
    In {
        expr: Box<Expr>,
        values: Vec<Literal>,
    },
    Column(String),
    Lit(Literal),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Null,
}
