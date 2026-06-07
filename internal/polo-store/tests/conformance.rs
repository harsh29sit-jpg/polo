/// Conformance tests — every assertion runs against both MemoryStore and SqliteStore
/// so bugs in one implementation can't hide behind the other.
use chrono::Utc;
use polo_core::{
    clock::Clock,
    db::{RecordParams, RetractParams, ScanQuery},
    fact::{Attr, BranchName, EntityId, FactId, Namespace, TxId, Value},
    gc::GcParams,
    merge::{DiffParams, MergeParams},
    namespace::{MergePolicy, NamespaceOpts},
    Store,
};
use polo_store::{MemoryStore, SqliteStore};
use std::sync::Arc;

// ── helpers ──────────────────────────────────────────────────────────────────

fn mem() -> Arc<dyn Store> {
    Arc::new(MemoryStore::new())
}

fn sqlite() -> Arc<dyn Store> {
    Arc::new(SqliteStore::open_in_memory().expect("sqlite in-memory"))
}

fn stores() -> Vec<(&'static str, Arc<dyn Store>)> {
    vec![("memory", mem()), ("sqlite", sqlite())]
}

fn clk() -> Clock {
    Clock::new()
}

fn default_ns() -> Namespace {
    Namespace::default()
}

fn main_branch() -> BranchName {
    BranchName::main()
}

async fn record_one(
    store: &Arc<dyn Store>,
    entity: &str,
    attr: &str,
    value: Value,
) -> FactId {
    let ts = clk().tick();
    let tx_id = TxId::new();
    store
        .record(RecordParams {
            namespace: default_ns(),
            entity: EntityId::new(entity),
            attr: Attr::new(attr),
            value,
            branch: main_branch(),
            author: None,
            message: None,
            valid_from: Utc::now(),
            valid_to: None,
            tx_time: ts,
            tx_id,
            caused_by: None,
            idempotency_key: None,
        })
        .await
        .expect("record")
        .fact_id
}

// ── record & get_fact ─────────────────────────────────────────────────────────

#[tokio::test]
async fn record_and_retrieve() {
    for (name, store) in stores() {
        let fid = record_one(&store, "user/1", "name", Value::Str("alice".into())).await;
        let fact = store.get_fact(fid).await.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(fact.entity.as_str(), "user/1", "{name}");
        assert_eq!(fact.attr.as_str(), "name", "{name}");
        assert!(matches!(fact.value, Value::Str(s) if s == "alice"), "{name}");
    }
}

#[tokio::test]
async fn get_fact_missing_returns_error() {
    for (name, store) in stores() {
        let missing = FactId::new();
        assert!(store.get_fact(missing).await.is_err(), "{name}: expected error for unknown fact");
    }
}

// ── typed values ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn typed_values_roundtrip() {
    let cases: Vec<(&str, Value)> = vec![
        ("str", Value::Str("hello".into())),
        ("int", Value::Int(42)),
        ("float", Value::Float(3.14)),
        ("bool_t", Value::Bool(true)),
        ("bool_f", Value::Bool(false)),
        ("null", Value::Null),
        ("json", Value::Json(serde_json::json!({"x": 1}))),
    ];

    for (name, store) in stores() {
        for (attr, val) in &cases {
            let fid = record_one(&store, "e/1", attr, val.clone()).await;
            let fact = store.get_fact(fid).await.unwrap_or_else(|e| panic!("{name}/{attr}: {e}"));
            assert_eq!(&fact.value, val, "{name}/{attr}");
        }
    }
}

// ── retract ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn retract_marks_fact() {
    for (name, store) in stores() {
        let fid = record_one(&store, "u/1", "age", Value::Int(30)).await;

        let tx_time = clk().tick();
        let tx_id = TxId::new();
        store
            .retract(
                fid.clone(),
                RetractParams {
                    namespace: default_ns(),
                    branch: main_branch(),
                    author: None,
                    message: None,
                    tx_time,
                    tx_id,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        let fact = store.get_fact(fid).await.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(fact.retracted, "{name}: fact should be retracted");
    }
}

#[tokio::test]
async fn retract_unknown_returns_error() {
    for (name, store) in stores() {
        let err = store
            .retract(
                FactId::new(),
                RetractParams {
                    namespace: default_ns(),
                    branch: main_branch(),
                    author: None,
                    message: None,
                    tx_time: clk().tick(),
                    tx_id: TxId::new(),
                },
            )
            .await;
        assert!(err.is_err(), "{name}: expected error for unknown fact");
    }
}

// ── scan ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn scan_filters_retracted_by_default() {
    for (name, store) in stores() {
        let fid = record_one(&store, "u/1", "x", Value::Int(1)).await;

        store
            .retract(
                fid,
                RetractParams {
                    namespace: default_ns(),
                    branch: main_branch(),
                    author: None,
                    message: None,
                    tx_time: clk().tick(),
                    tx_id: TxId::new(),
                },
            )
            .await
            .unwrap();

        let live = store
            .scan(ScanQuery {
                namespace: default_ns(),
                branch: main_branch(),
                ..Default::default()
            })
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        assert!(live.is_empty(), "{name}: retracted fact should not appear in default scan");
    }
}

#[tokio::test]
async fn scan_include_retracted() {
    for (name, store) in stores() {
        let fid = record_one(&store, "u/2", "y", Value::Bool(true)).await;
        store
            .retract(
                fid,
                RetractParams {
                    namespace: default_ns(),
                    branch: main_branch(),
                    author: None,
                    message: None,
                    tx_time: clk().tick(),
                    tx_id: TxId::new(),
                },
            )
            .await
            .unwrap();

        let all = store
            .scan(ScanQuery {
                namespace: default_ns(),
                branch: main_branch(),
                include_retracted: true,
                ..Default::default()
            })
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        assert_eq!(all.len(), 1, "{name}");
        assert!(all[0].retracted, "{name}");
    }
}

#[tokio::test]
async fn scan_limit_and_offset() {
    for (name, store) in stores() {
        for i in 0..5i64 {
            record_one(&store, &format!("e/{i}"), "n", Value::Int(i)).await;
        }

        let page = store
            .scan(ScanQuery {
                namespace: default_ns(),
                branch: main_branch(),
                limit: Some(2),
                offset: Some(1),
                ..Default::default()
            })
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        assert_eq!(page.len(), 2, "{name}");
    }
}

// ── idempotency ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn idempotency_key_deduplicates() {
    for (name, store) in stores() {
        let ts = clk().tick();
        let key = "ikey-1".to_string();

        let r1 = store
            .record(RecordParams {
                namespace: default_ns(),
                entity: EntityId::new("e/1"),
                attr: Attr::new("v"),
                value: Value::Int(10),
                branch: main_branch(),
                author: None,
                message: None,
                valid_from: Utc::now(),
                valid_to: None,
                tx_time: ts,
                tx_id: TxId::new(),
                caused_by: None,
                idempotency_key: Some(key.clone()),
            })
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        let r2 = store
            .record(RecordParams {
                namespace: default_ns(),
                entity: EntityId::new("e/1"),
                attr: Attr::new("v"),
                value: Value::Int(99),
                branch: main_branch(),
                author: None,
                message: None,
                valid_from: Utc::now(),
                valid_to: None,
                tx_time: clk().tick(),
                tx_id: TxId::new(),
                caused_by: None,
                idempotency_key: Some(key),
            })
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        assert!(r2.was_duplicate, "{name}");
        assert_eq!(r1.fact_id, r2.fact_id, "{name}");
    }
}

// ── namespaces ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn create_and_get_namespace() {
    for (name, store) in stores() {
        let ns = Namespace::new("orders");
        store
            .create_namespace(
                ns.clone(),
                NamespaceOpts {
                    merge_policy: MergePolicy::LastWriteWins,
                    schema: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        let info = store.get_namespace(&ns).await.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(info.name, ns, "{name}");
        assert_eq!(info.merge_policy, MergePolicy::LastWriteWins, "{name}");
    }
}

#[tokio::test]
async fn duplicate_namespace_errors() {
    for (name, store) in stores() {
        let ns = Namespace::new("dup-ns");
        let opts = NamespaceOpts { merge_policy: MergePolicy::default(), schema: None };
        store.create_namespace(ns.clone(), opts.clone()).await.unwrap();
        let err = store.create_namespace(ns, opts).await;
        assert!(err.is_err(), "{name}: expected duplicate namespace error");
    }
}

// ── branches ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn create_and_list_branches() {
    for (name, store) in stores() {
        let ns = default_ns();
        let fork = BranchName::new("feat");
        store
            .create_branch(&ns, fork.clone(), main_branch(), clk().tick())
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        let branches = store.list_branches(&ns).await.unwrap_or_else(|e| panic!("{name}: {e}"));
        let names: Vec<_> = branches.iter().map(|b| b.name.as_str()).collect();
        assert!(names.contains(&"main"), "{name}");
        assert!(names.contains(&"feat"), "{name}");
    }
}

#[tokio::test]
async fn delete_branch() {
    for (name, store) in stores() {
        let ns = default_ns();
        let branch = BranchName::new("to-delete");
        store
            .create_branch(&ns, branch.clone(), main_branch(), clk().tick())
            .await
            .unwrap();
        store.delete_branch(&ns, &branch).await.unwrap_or_else(|e| panic!("{name}: {e}"));

        let branches = store.list_branches(&ns).await.unwrap();
        assert!(!branches.iter().any(|b| b.name == branch), "{name}");
    }
}

#[tokio::test]
async fn cannot_delete_root_branch() {
    for (name, store) in stores() {
        let err = store.delete_branch(&default_ns(), &main_branch()).await;
        assert!(err.is_err(), "{name}: should not be able to delete root branch");
    }
}

// ── transactions ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn get_tx_after_record() {
    for (name, store) in stores() {
        let ts = clk().tick();
        let tx_id = TxId::new();
        store
            .record(RecordParams {
                namespace: default_ns(),
                entity: EntityId::new("e/1"),
                attr: Attr::new("a"),
                value: Value::Str("v".into()),
                branch: main_branch(),
                author: Some("alice".into()),
                message: Some("init".into()),
                valid_from: Utc::now(),
                valid_to: None,
                tx_time: ts,
                tx_id: tx_id.clone(),
                caused_by: None,
                idempotency_key: None,
            })
            .await
            .unwrap();

        let tx = store.get_tx(&tx_id).await.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(tx.id, tx_id, "{name}");
        assert_eq!(tx.author.as_deref(), Some("alice"), "{name}");
    }
}

// ── gc ────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn gc_removes_retracted_facts() {
    for (name, store) in stores() {
        let fid = record_one(&store, "u/1", "z", Value::Null).await;
        store
            .retract(
                fid,
                RetractParams {
                    namespace: default_ns(),
                    branch: main_branch(),
                    author: None,
                    message: None,
                    tx_time: clk().tick(),
                    tx_id: TxId::new(),
                },
            )
            .await
            .unwrap();

        let report = store
            .gc(GcParams { before_ms: None, branch: None, dry_run: false })
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        assert_eq!(report.facts_removed, 1, "{name}");

        let all = store
            .scan(ScanQuery {
                namespace: default_ns(),
                branch: main_branch(),
                include_retracted: true,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(all.is_empty(), "{name}: fact should be purged after gc");
    }
}

#[tokio::test]
async fn gc_dry_run_does_not_delete() {
    for (name, store) in stores() {
        let fid = record_one(&store, "u/1", "dry", Value::Null).await;
        store
            .retract(
                fid,
                RetractParams {
                    namespace: default_ns(),
                    branch: main_branch(),
                    author: None,
                    message: None,
                    tx_time: clk().tick(),
                    tx_id: TxId::new(),
                },
            )
            .await
            .unwrap();

        let report = store
            .gc(GcParams { before_ms: None, branch: None, dry_run: true })
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        assert!(report.dry_run, "{name}");
        assert_eq!(report.facts_removed, 1, "{name}");

        let all = store
            .scan(ScanQuery {
                namespace: default_ns(),
                branch: main_branch(),
                include_retracted: true,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(all.len(), 1, "{name}: fact should survive dry-run gc");
    }
}

// ── stats ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn stats_counts_correctly() {
    for (name, store) in stores() {
        record_one(&store, "e/1", "a", Value::Int(1)).await;
        record_one(&store, "e/2", "b", Value::Int(2)).await;

        let stats = store.stats().await.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(stats.facts, 2, "{name}");
        assert!(stats.transactions >= 2, "{name}");
        assert!(stats.namespaces >= 1, "{name}");
        assert!(stats.branches >= 1, "{name}");
    }
}

// ── list_entities / list_attrs ────────────────────────────────────────────────

#[tokio::test]
async fn list_entities_and_attrs() {
    for (name, store) in stores() {
        record_one(&store, "u/1", "name", Value::Str("a".into())).await;
        record_one(&store, "u/1", "age", Value::Int(30)).await;
        record_one(&store, "u/2", "name", Value::Str("b".into())).await;

        let entities = store.list_entities(&default_ns(), &main_branch()).await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(entities.len(), 2, "{name}");

        let attrs = store.list_attrs(&default_ns(), &main_branch(), &EntityId::new("u/1")).await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(attrs.len(), 2, "{name}");
    }
}

// ── tags ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn tags_crud() {
    for (name, store) in stores() {
        let tx_id = TxId::new();
        store.put_tag("release-1.0", &tx_id).await.unwrap_or_else(|e| panic!("{name}: {e}"));

        let got = store.get_tag("release-1.0").await.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(got, tx_id, "{name}");

        let all = store.list_tags().await.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(all.iter().any(|(l, _)| l == "release-1.0"), "{name}");

        store.delete_tag("release-1.0").await.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(store.get_tag("release-1.0").await.is_err(), "{name}");
    }
}

#[tokio::test]
async fn tag_overwrite() {
    for (name, store) in stores() {
        let tx1 = TxId::new();
        let tx2 = TxId::new();
        store.put_tag("v1", &tx1).await.unwrap();
        store.put_tag("v1", &tx2).await.unwrap();
        let got = store.get_tag("v1").await.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(got, tx2, "{name}: put_tag should overwrite");
    }
}

// ── diff ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn diff_between_branches() {
    for (name, store) in stores() {
        let ns = default_ns();
        let feat = BranchName::new("diff-feat");
        store
            .create_branch(&ns, feat.clone(), main_branch(), clk().tick())
            .await
            .unwrap();

        // Record on main
        let ts = clk().tick();
        store
            .record(RecordParams {
                namespace: ns.clone(),
                entity: EntityId::new("u/1"),
                attr: Attr::new("role"),
                value: Value::Str("admin".into()),
                branch: main_branch(),
                author: None,
                message: None,
                valid_from: Utc::now(),
                valid_to: None,
                tx_time: ts,
                tx_id: TxId::new(),
                caused_by: None,
                idempotency_key: None,
            })
            .await
            .unwrap();

        let entries = store
            .diff(DiffParams {
                namespace: ns.clone(),
                source: main_branch(),
                target: feat.clone(),
            })
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        assert_eq!(entries.len(), 1, "{name}");
        assert!(entries[0].target.is_none(), "{name}: target branch has no fact");
    }
}

// ── merge ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn merge_last_write_wins() {
    for (name, store) in stores() {
        let ns = Namespace::new(format!("merge-test-{name}").as_str());
        store
            .create_namespace(
                ns.clone(),
                NamespaceOpts { merge_policy: MergePolicy::LastWriteWins, schema: None },
            )
            .await
            .unwrap();

        let feat = BranchName::new("feat");
        store
            .create_branch(&ns, feat.clone(), main_branch(), clk().tick())
            .await
            .unwrap();

        // Write to feat only
        store
            .record(RecordParams {
                namespace: ns.clone(),
                entity: EntityId::new("u/1"),
                attr: Attr::new("score"),
                value: Value::Int(100),
                branch: feat.clone(),
                author: None,
                message: None,
                valid_from: Utc::now(),
                valid_to: None,
                tx_time: clk().tick(),
                tx_id: TxId::new(),
                caused_by: None,
                idempotency_key: None,
            })
            .await
            .unwrap();

        let result = store
            .merge(MergeParams {
                namespace: ns.clone(),
                source: feat.clone(),
                target: main_branch(),
                author: None,
                message: None,
                caused_by: None,
            })
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        assert_eq!(result.facts_applied, 1, "{name}");

        let facts = store
            .scan(ScanQuery {
                namespace: ns.clone(),
                branch: main_branch(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(facts.len(), 1, "{name}: merged fact should appear on main");
    }
}

// ── causal chain ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn causal_chain_preserved() {
    for (name, store) in stores() {
        let cause_id = TxId::new();
        let ts = clk().tick();

        let res = store
            .record(RecordParams {
                namespace: default_ns(),
                entity: EntityId::new("e/1"),
                attr: Attr::new("derived"),
                value: Value::Bool(true),
                branch: main_branch(),
                author: None,
                message: None,
                valid_from: Utc::now(),
                valid_to: None,
                tx_time: ts,
                tx_id: TxId::new(),
                caused_by: Some(cause_id.clone()),
                idempotency_key: None,
            })
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        let fact = store.get_fact(res.fact_id).await.unwrap();
        assert_eq!(fact.caused_by.as_ref(), Some(&cause_id), "{name}");
    }
}
