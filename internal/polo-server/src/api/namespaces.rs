use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use polo_core::{fact::Namespace, namespace::{MergePolicy, NamespaceOpts}};

use crate::{AppState, ApiError};

pub async fn list_namespaces(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let ns = state.db.list_namespaces().await?;
    Ok(Json(ns))
}

pub async fn get_namespace(
    State(state): State<AppState>,
    Path(ns): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let info = state.db.get_namespace(&Namespace::new(&ns)).await?;
    Ok(Json(info))
}

#[derive(Debug, Deserialize)]
pub struct CreateNamespaceRequest {
    pub name: String,
    pub merge_policy: Option<String>,
}

pub async fn create_namespace(
    State(state): State<AppState>,
    Json(body): Json<CreateNamespaceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let merge_policy = body
        .merge_policy
        .as_deref()
        .map(|s| s.parse::<MergePolicy>())
        .transpose()
        .map_err(|e| ApiError::bad_request(e))?
        .unwrap_or_default();

    state
        .db
        .create_namespace(Namespace::new(body.name), NamespaceOpts {
            merge_policy,
            schema: None,
        })
        .await?;

    Ok(StatusCode::CREATED)
}
