use chrono::Utc;
use polo_core::{
    clock::{Clock, Hlc},
    db::ScanQuery,
    fact::{Attr, BranchName, EntityId, Fact, FactId, Namespace, TxId, Value},
    gc::{GcParams, RetentionPolicy},
    schema::{AttrSpec, AttrType, Schema},
};
use std::collections::HashMap;

// ── clock ─────────────────────────────────────────────────────────────────────

#[test]
fn clock_monotonic() {
    let clk = Clock::new();
    let mut prev = clk.tick();
    for _ in 0..1000 {
        let next = clk.tick();
        assert!(next > prev, "clock must be strictly monotonic");
        prev = next;
    }
}

#[test]
fn clock_concurrent_monotonic() {
    use std::sync::Arc;
    let clk = Arc::new(Clock::new());
    let mut handles = Vec::new();
    for _ in 0..8 {
        let c = Arc::clone(&clk);
        handles.push(std::thread::spawn(move || {
            (0..500).map(|_| c.tick()).collect::<Vec<_>>()
        }));
    }
    let mut all: Vec<Hlc> = handles.into_iter().flat_map(|h| h.join().unwrap()).collect();
    all.sort();
    for w in all.windows(2) {
        assert!(w[0] <= w[1]);
    }
}

#[test]
fn clock_observe_advances() {
    let clk = Clock::new();
    // Force a very large external timestamp
    let future = Hlc(((Utc::now().timestamp_millis() as u64 + 1000) << 16) | 0);
    clk.observe(future);
    let t = clk.tick();
    assert!(t > future, "tick after observe should be > observed value");
}

#[test]
fn clock_display_roundtrip() {
    let clk = Clock::new();
    let t = clk.tick();
    let s = format!("{}", t);
    assert!(!s.is_empty());
}

#[test]
fn hlc_zero() {
    assert_eq!(Hlc::zero().0, 0);
}

// ── fact types ────────────────────────────────────────────────────────────────

#[test]
fn value_type_names() {
    assert_eq!(Value::Str("x".into()).type_name(), "str");
    assert_eq!(Value::Int(0).type_name(), "int");
    assert_eq!(Value::Float(0.0).type_name(), "float");
    assert_eq!(Value::Bool(false).type_name(), "bool");
    assert_eq!(Value::Json(serde_json::json!({})).type_name(), "json");
    assert_eq!(Value::Null.type_name(), "null");
}

#[test]
fn value_equality() {
    assert_eq!(Value::Int(42), Value::Int(42));
    assert_ne!(Value::Int(42), Value::Int(43));
    assert_ne!(Value::Str("a".into()), Value::Int(0));
}

#[test]
fn value_json_roundtrip() {
    let cases = vec![
        Value::Str("hello world".into()),
        Value::Int(-99),
        Value::Float(std::f64::consts::PI),
        Value::Bool(true),
        Value::Null,
        Value::Json(serde_json::json!({"a": [1, 2, 3]})),
    ];
    for v in &cases {
        let s = serde_json::to_string(v).unwrap();
        let back: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(&back, v);
    }
}

#[test]
fn namespace_default_is_default() {
    let ns = Namespace::default();
    assert_eq!(ns.as_str(), "default");
}

#[test]
fn branch_main_is_main() {
    let b = BranchName::main();
    assert_eq!(b.as_str(), "main");
}

#[test]
fn entity_id_as_str() {
    let e = EntityId::new("user/42");
    assert_eq!(e.as_str(), "user/42");
}

#[test]
fn fact_id_display_parses() {
    let fid = FactId::new();
    let s = fid.to_string();
    let parsed: FactId = s.parse().unwrap();
    assert_eq!(fid, parsed);
}

#[test]
fn tx_id_display_parses() {
    let tid = TxId::new();
    let s = tid.to_string();
    let parsed: TxId = s.parse().unwrap();
    assert_eq!(tid, parsed);
}

// ── schema validation ─────────────────────────────────────────────────────────

#[test]
fn schema_accepts_correct_type() {
    let mut attrs = HashMap::new();
    attrs.insert("age".into(), AttrSpec::new(AttrType::Int));
    let schema = Schema { attrs, strict: false };

    assert!(schema.validate(&Attr::new("age"), &Value::Int(30)).is_ok());
}

#[test]
fn schema_rejects_wrong_type() {
    let mut attrs = HashMap::new();
    attrs.insert("age".into(), AttrSpec::new(AttrType::Int));
    let schema = Schema { attrs, strict: false };

    let err = schema.validate(&Attr::new("age"), &Value::Str("thirty".into()));
    assert!(err.is_err());
}

#[test]
fn schema_strict_mode_rejects_unknown_attr() {
    let schema = Schema { attrs: HashMap::new(), strict: true };
    let err = schema.validate(&Attr::new("unknown"), &Value::Null);
    assert!(err.is_err());
}

#[test]
fn schema_non_strict_allows_unknown_attr() {
    let schema = Schema { attrs: HashMap::new(), strict: false };
    assert!(schema.validate(&Attr::new("unknown"), &Value::Null).is_ok());
}

#[test]
fn schema_any_accepts_all_types() {
    let mut attrs = HashMap::new();
    attrs.insert("flex".into(), AttrSpec::new(AttrType::Any));
    let schema = Schema { attrs, strict: false };

    let attr = Attr::new("flex");
    for v in [
        Value::Str("x".into()),
        Value::Int(1),
        Value::Float(1.0),
        Value::Bool(false),
        Value::Null,
        Value::Json(serde_json::json!(1)),
    ] {
        assert!(schema.validate(&attr, &v).is_ok(), "Any should accept {v:?}");
    }
}

// ── gc / retention ────────────────────────────────────────────────────────────

#[test]
fn gc_params_default() {
    let p = GcParams::default();
    assert!(p.before_ms.is_none());
    assert!(p.branch.is_none());
    assert!(!p.dry_run);
}

#[test]
fn retention_policy_default() {
    let p = RetentionPolicy::default();
    assert!(p.max_age_secs.is_none());
    assert!(p.max_versions.is_none());
    assert!(!p.only_retracted);
    assert!(p.branches.is_empty());
    assert!(!p.dry_run);
}

// ── pql ───────────────────────────────────────────────────────────────────────

fn make_fact(entity: &str, attr: &str, value: Value) -> Fact {
    Fact {
        id: FactId::new(),
        namespace: Namespace::default(),
        entity: EntityId::new(entity),
        attr: Attr::new(attr),
        value,
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
fn pql_parse_select_star() {
    let q = polo_core::pql::parse("SELECT * FROM default").unwrap();
    assert_eq!(q.namespace, "default");
    assert!(q.filter.is_none());
}

#[test]
fn pql_parse_with_entity_filter() {
    let q = polo_core::pql::parse(
        "SELECT entity, attr, value FROM default WHERE entity = 'user/1'",
    )
    .unwrap();
    assert!(q.filter.is_some());
}

#[test]
fn pql_parse_limit() {
    let q = polo_core::pql::parse("SELECT * FROM default LIMIT 10").unwrap();
    assert_eq!(q.limit, Some(10));
}

#[test]
fn pql_eval_entity_eq() {
    let facts = vec![
        make_fact("user/1", "name", Value::Str("alice".into())),
        make_fact("user/2", "name", Value::Str("bob".into())),
    ];
    let result = polo_core::pql::run(
        "SELECT entity, value FROM default WHERE entity = 'user/1'",
        BranchName::main(),
        facts,
    )
    .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["entity"], "user/1");
}

#[test]
fn pql_eval_attr_filter() {
    let facts = vec![
        make_fact("u/1", "name", Value::Str("alice".into())),
        make_fact("u/1", "age", Value::Int(30)),
    ];
    let result = polo_core::pql::run(
        "SELECT entity FROM default WHERE attr = 'age'",
        BranchName::main(),
        facts,
    )
    .unwrap();
    assert_eq!(result.len(), 1);
}

#[test]
fn pql_eval_like_prefix() {
    let facts = vec![
        make_fact("user/1", "x", Value::Null),
        make_fact("product/1", "x", Value::Null),
        make_fact("user/2", "x", Value::Null),
    ];
    let result = polo_core::pql::run(
        "SELECT entity FROM default WHERE entity LIKE 'user/%'",
        BranchName::main(),
        facts,
    )
    .unwrap();
    assert_eq!(result.len(), 2);
}

#[test]
fn pql_eval_limit() {
    let facts: Vec<Fact> = (0..10)
        .map(|i| make_fact(&format!("e/{i}"), "v", Value::Int(i)))
        .collect();
    let result = polo_core::pql::run(
        "SELECT entity FROM default LIMIT 3",
        BranchName::main(),
        facts,
    )
    .unwrap();
    assert_eq!(result.len(), 3);
}

#[test]
fn pql_invalid_syntax_returns_error() {
    assert!(polo_core::pql::parse("SELECTFROM broken !!").is_err());
}

// ── scan query defaults ───────────────────────────────────────────────────────

#[test]
fn scan_query_default_fields() {
    let q = ScanQuery::default();
    assert!(q.entity.is_none());
    assert!(q.attr.is_none());
    assert!(q.asof_tx.is_none());
    assert!(q.asof_valid.is_none());
    assert!(!q.include_retracted);
    assert!(q.limit.is_none());
    assert!(q.offset.is_none());
}

// ── error helpers ─────────────────────────────────────────────────────────────

#[test]
fn error_is_not_found() {
    use polo_core::Error;
    assert!(Error::FactNotFound(FactId::new()).is_not_found());
    assert!(Error::TxNotFound(TxId::new()).is_not_found());
    assert!(Error::NamespaceNotFound(Namespace::default()).is_not_found());
    assert!(Error::BranchNotFound(BranchName::main(), Namespace::default()).is_not_found());
    assert!(Error::TagNotFound("x".into()).is_not_found());
    assert!(!Error::other("misc").is_not_found());
}
