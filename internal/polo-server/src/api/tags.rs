use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use polo_core::fact::TxId;

use crate::{ApiError, AppState};

#[derive(Debug, Serialize)]
pub struct TagEntry {
    pub label: String,
    pub tx_id: String,
}

#[derive(Debug, Deserialize)]
pub struct PutTagBody {
    pub tx_id: String,
}

pub async fn list_tags(
    State(s): State<AppState>,
) -> Result<Json<Vec<TagEntry>>, ApiError> {
    let pairs = s.db.list_tags().await?;
    Ok(Json(
        pairs
            .into_iter()
            .map(|(label, tx_id)| TagEntry {
                label,
                tx_id: tx_id.to_string(),
            })
            .collect(),
    ))
}

pub async fn get_tag(
    State(s): State<AppState>,
    Path(label): Path<String>,
) -> Result<Json<TagEntry>, ApiError> {
    let tx_id = s.db.get_tag(&label).await?;
    Ok(Json(TagEntry {
        label,
        tx_id: tx_id.to_string(),
    }))
}

pub async fn put_tag(
    State(s): State<AppState>,
    Path(label): Path<String>,
    Json(body): Json<PutTagBody>,
) -> Result<StatusCode, ApiError> {
    let tx_id: TxId = body
        .tx_id
        .parse()
        .map_err(|_| ApiError::bad_request("invalid tx_id"))?;
    s.db.put_tag(&label, &tx_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_tag(
    State(s): State<AppState>,
    Path(label): Path<String>,
) -> Result<StatusCode, ApiError> {
    s.db.delete_tag(&label).await?;
    Ok(StatusCode::NO_CONTENT)
}
