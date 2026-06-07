pub mod ast;
pub mod eval;
pub mod lexer;
pub mod parser;
pub mod token;

use crate::{error::Error, fact::BranchName};

pub use ast::Query;
pub use eval::Evaluator;

pub fn parse(input: &str) -> Result<Query, Error> {
    let tokens = lexer::Lexer::new(input).tokenize()?;
    parser::Parser::new(tokens).parse()
}

pub fn run(
    input: &str,
    branch: BranchName,
    facts: Vec<crate::fact::Fact>,
) -> Result<Vec<serde_json::Value>, Error> {
    let query = parse(input)?;
    Evaluator::new(&query, branch).eval(facts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fact::{Attr, BranchName, EntityId, Fact, FactId, Namespace, TxId, Value};
    use crate::clock::Hlc;
    use chrono::Utc;

    fn make_fact(entity: &str, attr: &str, value: &str) -> Fact {
        Fact {
            id: FactId::new(),
            namespace: Namespace::default(),
            entity: EntityId::new(entity),
            attr: Attr::new(attr),
            value: Value::Str(value.to_owned()),
            valid_from: Utc::now(),
            valid_to: None,
            tx_id: TxId::new(),
            tx_time: Hlc::zero(),
            branch: BranchName::main(),
            author: None,
            retracted: false,
            caused_by: None,
        }
    }

    #[test]
    fn parse_basic_select() {
        let q = parse("SELECT entity, attr, value FROM default WHERE entity = 'user/1'").unwrap();
        assert_eq!(q.namespace, "default");
        assert!(q.filter.is_some());
    }

    #[test]
    fn eval_filter_eq() {
        let facts = vec![
            make_fact("user/1", "name", "alice"),
            make_fact("user/2", "name", "bob"),
        ];
        let results = run(
            "SELECT entity, value FROM default WHERE entity = 'user/1'",
            BranchName::main(),
            facts,
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["entity"], "user/1");
    }

    #[test]
    fn eval_like() {
        let facts = vec![
            make_fact("user/1", "name", "alice"),
            make_fact("product/1", "name", "widget"),
        ];
        let results = run(
            "SELECT entity FROM default WHERE entity LIKE 'user/%'",
            BranchName::main(),
            facts,
        )
        .unwrap();
        assert_eq!(results.len(), 1);
    }
}
