use axum::{extract::State, Json};

use polo_core::stats::StoreStats;

use crate::{ApiError, AppState};

pub async fn get_stats(State(s): State<AppState>) -> Result<Json<StoreStats>, ApiError> {
    let stats = s.db.stats().await?;
    Ok(Json(stats))
}
