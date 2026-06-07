use std::sync::Arc;

use axum::body::{to_bytes, Body};
use http::{Method, Request, StatusCode};
use polo_core::{Db, fact::TxId};
use polo_server::{api::stream::EventBus, build_router, AppState};
use polo_store::MemoryStore;
use serde_json::Value as Json;
use tower::ServiceExt;

// ── helpers ───────────────────────────────────────────────────────────────────

fn test_state() -> AppState {
    let store = Arc::new(MemoryStore::new());
    let db = Arc::new(Db::new(store));
    let bus = Arc::new(EventBus::new(256));
    AppState { db, token: None, bus }
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn post_json(uri: &str, body: Json) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn put_json(uri: &str, body: Json) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn delete(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::DELETE)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

async fn body_json(b: Body) -> Json {
    let bytes = to_bytes(b, 1 << 20).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// ── health ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn healthz_returns_ok() {
    let app = build_router(test_state(), None);
    let resp = app.oneshot(get("/healthz")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn version_returns_json() {
    let app = build_router(test_state(), None);
    let resp = app.oneshot(get("/version")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = body_json(resp.into_body()).await;
    assert!(j.get("version").is_some());
}

// ── namespaces ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_namespaces_includes_default() {
    let app = build_router(test_state(), None);
    let resp = app.oneshot(get("/namespaces")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let arr = body_json(resp.into_body()).await;
    let arr = arr.as_array().unwrap();
    assert!(arr.iter().any(|ns| ns["name"] == "default"));
}

#[tokio::test]
async fn create_namespace_then_get() {
    let state = test_state();
    let app = build_router(state, None);

    let resp = app
        .clone()
        .oneshot(post_json(
            "/namespaces",
            serde_json::json!({ "name": "orders", "merge_policy": "last_write_wins" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let resp = app.oneshot(get("/namespaces/orders")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j = body_json(resp.into_body()).await;
    assert_eq!(j["name"], "orders");
}

// ── facts ─────────────────────────────────────────────────────────────────────

async fn record_fact(app: axum::Router, entity: &str, attr: &str, val: &str) -> Json {
    let body = serde_json::json!({
        "entity": entity,
        "attr": attr,
        "value": { "type": "str", "value": val },
        "branch": "main"
    });
    let resp = app
        .oneshot(post_json("/v1/default/facts", body))
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::CREATED || resp.status() == StatusCode::OK,
        "unexpected status {}",
        resp.status()
    );
    body_json(resp.into_body()).await
}

#[tokio::test]
async fn record_and_get_fact() {
    let state = test_state();
    let app = build_router(state, None);

    let result = record_fact(app.clone(), "user/1", "name", "alice").await;
    let fact_id = result["fact_id"].as_str().unwrap().to_string();

    let resp = app
        .oneshot(get(&format!("/v1/default/facts/{fact_id}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let fact = body_json(resp.into_body()).await;
    assert_eq!(fact["entity"], "user/1");
    assert_eq!(fact["attr"], "name");
}

#[tokio::test]
async fn get_unknown_fact_is_404() {
    let app = build_router(test_state(), None);
    let missing = TxId::new().to_string();
    let resp = app
        .oneshot(get(&format!("/v1/default/facts/{missing}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn retract_fact() {
    let state = test_state();
    let app = build_router(state, None);

    let result = record_fact(app.clone(), "u/1", "age", "30").await;
    let fact_id = result["fact_id"].as_str().unwrap().to_string();

    let resp = app
        .oneshot(delete(&format!("/v1/default/facts/{fact_id}?branch=main")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ── history / snapshot ────────────────────────────────────────────────────────

#[tokio::test]
async fn history_returns_all_versions() {
    let state = test_state();
    let app = build_router(state, None);

    record_fact(app.clone(), "user/1", "name", "alice").await;
    record_fact(app.clone(), "user/1", "name", "alicia").await;

    let resp = app
        .oneshot(get("/v1/default/history?entity=user/1&attr=name&branch=main"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let arr = body_json(resp.into_body()).await;
    assert!(arr.as_array().unwrap().len() >= 2);
}

#[tokio::test]
async fn snapshot_returns_entity_attrs() {
    let state = test_state();
    let app = build_router(state, None);

    record_fact(app.clone(), "user/2", "name", "bob").await;
    record_fact(app.clone(), "user/2", "email", "bob@example.com").await;

    let resp = app
        .oneshot(get("/v1/default/snapshot/user%2F2?branch=main"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let facts = body_json(resp.into_body()).await;
    assert_eq!(facts.as_array().unwrap().len(), 2);
}

// ── branches ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_branches_includes_main() {
    let app = build_router(test_state(), None);
    let resp = app.oneshot(get("/v1/default/branches")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let branches = body_json(resp.into_body()).await;
    assert!(branches
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["name"] == "main"));
}

#[tokio::test]
async fn create_and_delete_branch() {
    let state = test_state();
    let app = build_router(state, None);

    let resp = app
        .clone()
        .oneshot(post_json(
            "/v1/default/branches",
            serde_json::json!({ "name": "feat-x", "from": "main" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let resp = app
        .clone()
        .oneshot(get("/v1/default/branches/feat-x"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = app
        .oneshot(delete("/v1/default/branches/feat-x"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

// ── tags ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn tags_lifecycle() {
    let state = test_state();
    let app = build_router(state, None);

    let tx_id = TxId::new().to_string();

    // PUT
    let resp = app
        .clone()
        .oneshot(put_json(
            "/v1/tags/v1.0",
            serde_json::json!({ "tx_id": tx_id }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // GET
    let resp = app
        .clone()
        .oneshot(get("/v1/tags/v1.0"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let tag = body_json(resp.into_body()).await;
    assert_eq!(tag["label"], "v1.0");
    assert_eq!(tag["tx_id"], tx_id);

    // LIST
    let resp = app.clone().oneshot(get("/v1/tags")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let tags = body_json(resp.into_body()).await;
    assert!(tags
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["label"] == "v1.0"));

    // DELETE
    let resp = app.oneshot(delete("/v1/tags/v1.0")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

// ── stats ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn stats_endpoint() {
    let app = build_router(test_state(), None);
    let resp = app.oneshot(get("/v1/stats")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let stats = body_json(resp.into_body()).await;
    assert!(stats.get("facts").is_some());
    assert!(stats.get("namespaces").is_some());
}

// ── PQL query ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn pql_query_returns_rows() {
    let state = test_state();
    let app = build_router(state, None);

    record_fact(app.clone(), "u/1", "role", "admin").await;

    let resp = app
        .oneshot(post_json(
            "/v1/default/query",
            serde_json::json!({
                "pql": "SELECT entity, attr, value FROM default WHERE attr = 'role'",
                "branch": "main"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let result = body_json(resp.into_body()).await;
    let rows = result["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["entity"], "u/1");
}

// ── auth ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn token_auth_rejects_missing_token() {
    let mut state = test_state();
    state.token = Some("secret".into());
    let app = build_router(state, None);

    let resp = app.oneshot(get("/v1/stats")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn token_auth_accepts_correct_token() {
    let mut state = test_state();
    state.token = Some("secret".into());
    let app = build_router(state, None);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/stats")
        .header("authorization", "Bearer secret")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
