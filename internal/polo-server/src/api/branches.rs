use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use polo_core::fact::{BranchName, Namespace};

use crate::{AppState, ApiError};

#[derive(Debug, Deserialize)]
pub struct CreateBranchRequest {
    pub name: String,
    pub from: Option<String>,
}

pub async fn list_branches(
    State(state): State<AppState>,
    Path(ns): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let branches = state.db.list_branches(&Namespace::new(&ns)).await?;
    Ok(Json(branches))
}

pub async fn create_branch(
    State(state): State<AppState>,
    Path(ns): Path<String>,
    Json(body): Json<CreateBranchRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let fork_from = body
        .from
        .as_deref()
        .map(BranchName::new)
        .unwrap_or_default();

    state
        .db
        .create_branch(&Namespace::new(&ns), BranchName::new(body.name), fork_from)
        .await?;

    Ok(StatusCode::CREATED)
}

pub async fn delete_branch(
    State(state): State<AppState>,
    Path((ns, branch)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .db
        .delete_branch(&Namespace::new(&ns), &BranchName::new(branch))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_branch(
    State(state): State<AppState>,
    Path((ns, branch)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let info = state
        .db
        .get_branch(&Namespace::new(&ns), &BranchName::new(branch))
        .await?;
    Ok(Json(info))
}

#[derive(Debug, Deserialize)]
pub struct MergeRequest {
    pub source: String,
    pub target: String,
    pub author: Option<String>,
    pub message: Option<String>,
    pub caused_by: Option<String>,
}

pub async fn merge(
    State(state): State<AppState>,
    Path(ns): Path<String>,
    Json(body): Json<MergeRequest>,
) -> Result<impl IntoResponse, ApiError> {
    use polo_core::fact::TxId;
    use polo_core::merge::MergeParams;

    let caused_by = body
        .caused_by
        .as_deref()
        .map(|s| s.parse::<TxId>())
        .transpose()
        .map_err(|_| ApiError::bad_request("invalid caused_by"))?;

    let result = state
        .db
        .merge(MergeParams {
            namespace: Namespace::new(&ns),
            source: BranchName::new(body.source),
            target: BranchName::new(body.target),
            author: body.author,
            message: body.message,
            caused_by,
        })
        .await?;

    Ok(Json(result))
}

#[derive(Debug, Deserialize)]
pub struct DiffQuery {
    pub source: String,
    pub target: Option<String>,
}

pub async fn diff(
    State(state): State<AppState>,
    Path(ns): Path<String>,
    axum::extract::Query(q): axum::extract::Query<DiffQuery>,
) -> Result<impl IntoResponse, ApiError> {
    use polo_core::merge::DiffParams;

    let target = q.target.as_deref().map(BranchName::new).unwrap_or_default();
    let entries = state
        .db
        .diff(DiffParams {
            namespace: Namespace::new(&ns),
            source: BranchName::new(q.source),
            target,
        })
        .await?;

    Ok(Json(entries))
}
