use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use polo_core::{
    db::{RecordOpts, RetractOpts},
    fact::{Attr, BranchName, EntityId, FactId, Namespace, TxId, Value},
};

use crate::{AppState, ApiError};

#[derive(Debug, Deserialize)]
pub struct RecordRequest {
    pub entity: String,
    pub attr: String,
    pub value: Value,
    pub branch: Option<String>,
    pub author: Option<String>,
    pub message: Option<String>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_to: Option<DateTime<Utc>>,
    pub caused_by: Option<String>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RecordResponse {
    pub fact_id: String,
    pub tx_id: String,
    pub was_duplicate: bool,
}

pub async fn record_fact(
    State(state): State<AppState>,
    Path(ns): Path<String>,
    Json(body): Json<RecordRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let namespace = Namespace::new(&ns);
    let branch = body
        .branch
        .as_deref()
        .map(BranchName::new)
        .unwrap_or_default();

    let caused_by = body
        .caused_by
        .as_deref()
        .map(|s| s.parse::<TxId>())
        .transpose()
        .map_err(|_| ApiError::bad_request("invalid caused_by TxId"))?;

    let res = state
        .db
        .record(
            namespace,
            EntityId::new(body.entity),
            Attr::new(body.attr),
            body.value,
            branch,
            RecordOpts {
                author: body.author,
                message: body.message,
                valid_from: body.valid_from,
                valid_to: body.valid_to,
                caused_by,
                idempotency_key: body.idempotency_key,
            },
        )
        .await?;

    let status = if res.was_duplicate {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };

    Ok((
        status,
        Json(RecordResponse {
            fact_id: res.fact_id.to_string(),
            tx_id: res.tx_id.to_string(),
            was_duplicate: res.was_duplicate,
        }),
    ))
}

pub async fn get_fact(
    State(state): State<AppState>,
    Path((_ns, fact_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let id: FactId = fact_id
        .parse()
        .map_err(|_| ApiError::bad_request("invalid fact id"))?;
    let fact = state.db.get_fact(id).await?;
    Ok(Json(fact))
}

#[derive(Debug, Deserialize)]
pub struct RetractQuery {
    pub branch: Option<String>,
    pub author: Option<String>,
    pub message: Option<String>,
}

pub async fn retract_fact(
    State(state): State<AppState>,
    Path((ns, fact_id)): Path<(String, String)>,
    Query(q): Query<RetractQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let id: FactId = fact_id
        .parse()
        .map_err(|_| ApiError::bad_request("invalid fact id"))?;
    let branch = q.branch.as_deref().map(BranchName::new).unwrap_or_default();
    let tx_id = state
        .db
        .retract(
            Namespace::new(&ns),
            id,
            branch,
            RetractOpts {
                author: q.author,
                message: q.message,
            },
        )
        .await?;
    Ok(Json(serde_json::json!({ "tx_id": tx_id.to_string() })))
}

#[derive(Debug, Deserialize)]
pub struct AsofQuery {
    pub entity: String,
    pub attr: String,
    pub branch: Option<String>,
    pub at: Option<String>,
}

pub async fn asof(
    State(state): State<AppState>,
    Path(ns): Path<String>,
    Query(q): Query<AsofQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let branch = q.branch.as_deref().map(BranchName::new).unwrap_or_default();
    let at = q
        .at
        .as_deref()
        .map(|s| s.parse::<polo_core::Hlc>())
        .transpose()
        .map_err(|_| ApiError::bad_request("invalid HLC in 'at' — expected hex string"))?
        .unwrap_or_else(|| state.db.clock().now());

    let fact = state
        .db
        .asof(
            Namespace::new(&ns),
            branch,
            EntityId::new(q.entity),
            Attr::new(q.attr),
            at,
        )
        .await?;

    Ok(Json(fact))
}

#[derive(Debug, Deserialize)]
pub struct EffectiveQuery {
    pub entity: String,
    pub attr: String,
    pub branch: Option<String>,
    pub at: Option<DateTime<Utc>>,
}

pub async fn effective(
    State(state): State<AppState>,
    Path(ns): Path<String>,
    Query(q): Query<EffectiveQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let branch = q.branch.as_deref().map(BranchName::new).unwrap_or_default();
    let at = q.at.unwrap_or_else(Utc::now);

    let fact = state
        .db
        .effective(
            Namespace::new(&ns),
            branch,
            EntityId::new(q.entity),
            Attr::new(q.attr),
            at,
        )
        .await?;

    Ok(Json(fact))
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub entity: String,
    pub attr: String,
    pub branch: Option<String>,
}

pub async fn history(
    State(state): State<AppState>,
    Path(ns): Path<String>,
    Query(q): Query<HistoryQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let branch = q.branch.as_deref().map(BranchName::new).unwrap_or_default();
    let facts = state
        .db
        .history(
            Namespace::new(&ns),
            branch,
            EntityId::new(q.entity),
            Attr::new(q.attr),
        )
        .await?;
    Ok(Json(facts))
}

#[derive(Debug, Deserialize)]
pub struct SnapshotQuery {
    pub branch: Option<String>,
}

pub async fn snapshot(
    State(state): State<AppState>,
    Path((ns, entity)): Path<(String, String)>,
    Query(q): Query<SnapshotQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let branch = q.branch.as_deref().map(BranchName::new).unwrap_or_default();
    let facts = state
        .db
        .snapshot(Namespace::new(&ns), branch, EntityId::new(entity))
        .await?;
    Ok(Json(facts))
}

#[derive(Debug, Deserialize)]
pub struct TxListQuery {
    pub branch: Option<String>,
    pub limit: Option<usize>,
}

pub async fn list_tx(
    State(state): State<AppState>,
    Path(ns): Path<String>,
    Query(q): Query<TxListQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let branch = q.branch.as_deref().map(BranchName::new).unwrap_or_default();
    let limit = q.limit.unwrap_or(50).min(500);
    let txs = state
        .db
        .list_tx(&Namespace::new(&ns), &branch, limit)
        .await?;
    Ok(Json(txs))
}

pub async fn get_tx(
    State(state): State<AppState>,
    Path((_ns, tx_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let id: TxId = tx_id
        .parse()
        .map_err(|_| ApiError::bad_request("invalid tx id"))?;
    let tx = state.db.get_tx(&id).await?;
    Ok(Json(tx))
}
