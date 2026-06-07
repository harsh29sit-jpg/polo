use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;

use polo_core::fact::{BranchName, Namespace};

use crate::{ApiError, AppState};

#[derive(Debug, Deserialize)]
pub struct EntitiesQuery {
    #[serde(default = "default_branch")]
    pub branch: String,
}

fn default_branch() -> String {
    "main".into()
}

pub async fn list_entities(
    State(s): State<AppState>,
    Path(ns): Path<String>,
    Query(q): Query<EntitiesQuery>,
) -> Result<Json<Vec<String>>, ApiError> {
    let ns = Namespace::new(&ns);
    let branch = BranchName::new(&q.branch);
    let entities = s.db.list_entities(&ns, &branch).await?;
    Ok(Json(entities.into_iter().map(|e| e.0).collect()))
}
