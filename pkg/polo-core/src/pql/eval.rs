use super::ast::{BinOp, Column, Direction, Expr, Literal, Query};
use crate::{
    db::ScanQuery,
    error::Error,
    fact::{Attr, BranchName, EntityId, Fact, Namespace, Value},
};

/// Converts a PQL query into a ScanQuery for the store and post-filters results.
pub struct Evaluator<'a> {
    query: &'a Query,
    branch: BranchName,
}

impl<'a> Evaluator<'a> {
    pub fn new(query: &'a Query, branch: BranchName) -> Self {
        Self { query, branch }
    }

    pub fn to_scan_query(&self) -> ScanQuery {
        // Extract simple equality predicates that the store can handle natively.
        let (entity, attr) = extract_simple_predicates(self.query.filter.as_ref());

        ScanQuery {
            namespace: Namespace::new(&self.query.namespace),
            branch: self.branch.clone(),
            entity,
            attr,
            asof_tx: self.query.asof,
            asof_valid: self.query.effective_at,
            include_retracted: false,
            limit: if self.query.filter.is_none() {
                self.query.limit
            } else {
                None // fetch more and filter client-side
            },
            offset: if self.query.filter.is_none() {
                self.query.offset
            } else {
                None
            },
        }
    }

    pub fn eval(&self, facts: Vec<Fact>) -> Result<Vec<serde_json::Value>, Error> {
        let mut results: Vec<Fact> = facts
            .into_iter()
            .filter(|f| {
                self.query
                    .filter
                    .as_ref()
                    .map(|e| eval_filter(e, f))
                    .unwrap_or(true)
            })
            .collect();

        // Sorting
        for ob in self.query.order_by.iter().rev() {
            results.sort_by(|a, b| {
                let ord = compare_col(&ob.column, a, b);
                if ob.direction == Direction::Desc {
                    ord.reverse()
                } else {
                    ord
                }
            });
        }

        // Pagination (only needed when we couldn't push it to the store)
        if self.query.filter.is_some() {
            let offset = self.query.offset.unwrap_or(0);
            let limit = self.query.limit.unwrap_or(usize::MAX);
            results = results.into_iter().skip(offset).take(limit).collect();
        }

        results.into_iter().map(|f| project(&self.query.columns, f)).collect()
    }
}

fn project(cols: &[Column], f: Fact) -> Result<serde_json::Value, Error> {
    use serde_json::{json, Map};

    let has_star = cols.iter().any(|c| matches!(c, Column::Star));
    let mut map = Map::new();

    if has_star {
        map.insert("id".into(), json!(f.id.to_string()));
        map.insert("namespace".into(), json!(f.namespace.0));
        map.insert("entity".into(), json!(f.entity.0));
        map.insert("attr".into(), json!(f.attr.0));
        map.insert("value".into(), value_to_json(&f.value));
        map.insert("valid_from".into(), json!(f.valid_from.to_rfc3339()));
        map.insert("valid_to".into(), f.valid_to.map(|t| json!(t.to_rfc3339())).unwrap_or(serde_json::Value::Null));
        map.insert("tx_id".into(), json!(f.tx_id.to_string()));
        map.insert("tx_time".into(), json!(f.tx_time.to_string()));
        map.insert("branch".into(), json!(f.branch.0));
        map.insert("author".into(), f.author.map(|a| json!(a)).unwrap_or(serde_json::Value::Null));
        map.insert("caused_by".into(), f.caused_by.map(|c| json!(c.to_string())).unwrap_or(serde_json::Value::Null));
    } else {
        for col in cols {
            if let Column::Named(name) = col {
                let v = match name.as_str() {
                    "id" => json!(f.id.to_string()),
                    "namespace" => json!(f.namespace.0),
                    "entity" => json!(f.entity.0),
                    "attr" => json!(f.attr.0),
                    "value" => value_to_json(&f.value),
                    "valid_from" => json!(f.valid_from.to_rfc3339()),
                    "valid_to" => f.valid_to.map(|t| json!(t.to_rfc3339())).unwrap_or(serde_json::Value::Null),
                    "tx_id" => json!(f.tx_id.to_string()),
                    "tx_time" => json!(f.tx_time.to_string()),
                    "branch" => json!(f.branch.0),
                    "author" => f.author.as_deref().map(|a| json!(a)).unwrap_or(serde_json::Value::Null),
                    "caused_by" => f.caused_by.as_ref().map(|c| json!(c.to_string())).unwrap_or(serde_json::Value::Null),
                    unknown => {
                        return Err(Error::Query(format!("unknown column '{unknown}'")));
                    }
                };
                map.insert(name.clone(), v);
            }
        }
    }

    Ok(serde_json::Value::Object(map))
}

fn value_to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Str(s) => serde_json::Value::String(s.clone()),
        Value::Int(n) => serde_json::json!(n),
        Value::Float(f) => serde_json::json!(f),
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Json(j) => j.clone(),
        Value::Null => serde_json::Value::Null,
    }
}

fn eval_filter(expr: &Expr, fact: &Fact) -> bool {
    match expr {
        Expr::And(a, b) => eval_filter(a, fact) && eval_filter(b, fact),
        Expr::Or(a, b) => eval_filter(a, fact) || eval_filter(b, fact),
        Expr::Not(inner) => !eval_filter(inner, fact),
        Expr::IsNull(e) => {
            if let Expr::Column(col) = e.as_ref() {
                col_is_null(col, fact)
            } else {
                false
            }
        }
        Expr::IsNotNull(e) => {
            if let Expr::Column(col) = e.as_ref() {
                !col_is_null(col, fact)
            } else {
                true
            }
        }
        Expr::Like { expr, pattern } => {
            if let Expr::Column(col) = expr.as_ref() {
                let s = col_str(col, fact);
                like_match(&s, pattern)
            } else {
                false
            }
        }
        Expr::In { expr, values } => {
            if let Expr::Column(col) = expr.as_ref() {
                values.iter().any(|v| col_lit_eq(col, v, fact))
            } else {
                false
            }
        }
        Expr::BinOp { op, left, right } => eval_binop(op, left, right, fact),
        Expr::Column(_) | Expr::Lit(_) => true,
    }
}

fn eval_binop(op: &BinOp, left: &Expr, right: &Expr, fact: &Fact) -> bool {
    let lv = resolve(left, fact);
    let rv = resolve(right, fact);

    match (lv, rv) {
        (Some(a), Some(b)) => match op {
            BinOp::Eq => a == b,
            BinOp::Ne => a != b,
            BinOp::Lt => compare_lits(&a, &b) == Some(std::cmp::Ordering::Less),
            BinOp::Le => matches!(
                compare_lits(&a, &b),
                Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
            ),
            BinOp::Gt => compare_lits(&a, &b) == Some(std::cmp::Ordering::Greater),
            BinOp::Ge => matches!(
                compare_lits(&a, &b),
                Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
            ),
        },
        _ => false,
    }
}

fn resolve(expr: &Expr, fact: &Fact) -> Option<Literal> {
    match expr {
        Expr::Column(col) => col_to_lit(col, fact),
        Expr::Lit(lit) => Some(lit.clone()),
        _ => None,
    }
}

fn col_to_lit(col: &str, fact: &Fact) -> Option<Literal> {
    match col {
        "entity" => Some(Literal::Str(fact.entity.0.clone())),
        "attr" => Some(Literal::Str(fact.attr.0.clone())),
        "branch" => Some(Literal::Str(fact.branch.0.clone())),
        "namespace" => Some(Literal::Str(fact.namespace.0.clone())),
        "author" => fact.author.as_ref().map(|a| Literal::Str(a.clone())),
        "value" => match &fact.value {
            Value::Str(s) => Some(Literal::Str(s.clone())),
            Value::Int(n) => Some(Literal::Int(*n)),
            Value::Float(f) => Some(Literal::Float(*f)),
            Value::Bool(b) => Some(Literal::Bool(*b)),
            _ => None,
        },
        _ => None,
    }
}

fn col_str(col: &str, fact: &Fact) -> String {
    col_to_lit(col, fact)
        .and_then(|l| if let Literal::Str(s) = l { Some(s) } else { None })
        .unwrap_or_default()
}

fn col_is_null(col: &str, fact: &Fact) -> bool {
    col_to_lit(col, fact).map(|l| l == Literal::Null).unwrap_or(true)
}

fn col_lit_eq(col: &str, lit: &Literal, fact: &Fact) -> bool {
    col_to_lit(col, fact).map(|l| &l == lit).unwrap_or(false)
}

fn compare_lits(a: &Literal, b: &Literal) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Literal::Int(x), Literal::Int(y)) => Some(x.cmp(y)),
        (Literal::Float(x), Literal::Float(y)) => x.partial_cmp(y),
        (Literal::Int(x), Literal::Float(y)) => (*x as f64).partial_cmp(y),
        (Literal::Float(x), Literal::Int(y)) => x.partial_cmp(&(*y as f64)),
        (Literal::Str(x), Literal::Str(y)) => Some(x.cmp(y)),
        (Literal::Bool(x), Literal::Bool(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

fn like_match(s: &str, pattern: &str) -> bool {
    let parts: Vec<&str> = pattern.split('%').collect();
    if parts.len() == 1 {
        return s == pattern;
    }
    let mut remaining = s;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            if !remaining.starts_with(part) {
                return false;
            }
            remaining = &remaining[part.len()..];
        } else if i == parts.len() - 1 {
            if !remaining.ends_with(part) {
                return false;
            }
        } else {
            match remaining.find(part) {
                Some(pos) => remaining = &remaining[pos + part.len()..],
                None => return false,
            }
        }
    }
    true
}

fn compare_col(col: &str, a: &Fact, b: &Fact) -> std::cmp::Ordering {
    match col {
        "valid_from" => a.valid_from.cmp(&b.valid_from),
        "tx_time" => a.tx_time.cmp(&b.tx_time),
        "entity" => a.entity.0.cmp(&b.entity.0),
        "attr" => a.attr.0.cmp(&b.attr.0),
        _ => std::cmp::Ordering::Equal,
    }
}

fn extract_simple_predicates(filter: Option<&Expr>) -> (Option<EntityId>, Option<Attr>) {
    let Some(expr) = filter else {
        return (None, None);
    };

    let mut entity = None;
    let mut attr = None;

    extract_from_expr(expr, &mut entity, &mut attr);
    (entity, attr)
}

fn extract_from_expr(expr: &Expr, entity: &mut Option<EntityId>, attr: &mut Option<Attr>) {
    match expr {
        Expr::And(a, b) => {
            extract_from_expr(a, entity, attr);
            extract_from_expr(b, entity, attr);
        }
        Expr::BinOp {
            op: BinOp::Eq,
            left,
            right,
        } => {
            if let (Expr::Column(col), Expr::Lit(Literal::Str(val))) =
                (left.as_ref(), right.as_ref())
            {
                match col.as_str() {
                    "entity" => *entity = Some(EntityId::new(val)),
                    "attr" => *attr = Some(Attr::new(val)),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}
