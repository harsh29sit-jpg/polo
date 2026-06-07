use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use polo_core::fact::{BranchName, Namespace};

use crate::{AppState, ApiError};

#[derive(Debug, Deserialize)]
pub struct QueryRequest {
    pub pql: String,
    pub branch: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct QueryResponse {
    pub rows: Vec<serde_json::Value>,
    pub count: usize,
}

pub async fn run_query(
    State(state): State<AppState>,
    Path(ns): Path<String>,
    Json(body): Json<QueryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let branch = body
        .branch
        .as_deref()
        .map(BranchName::new)
        .unwrap_or_default();

    let query =
        polo_core::pql::parse(&body.pql).map_err(|e| ApiError::bad_request(e.to_string()))?;

    let scan_q = polo_core::pql::Evaluator::new(&query, branch.clone()).to_scan_query();

    let facts = state.db.scan(scan_q).await?;
    let rows =
        polo_core::pql::Evaluator::new(&query, branch).eval(facts)?;

    let count = rows.len();
    Ok(Json(QueryResponse { rows, count }))
}
