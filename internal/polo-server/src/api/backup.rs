/// NDJSON dump / restore for a namespace.
///
/// Dump: GET /v1/:ns/dump  → one JSON object per line, each is a Fact.
/// Restore: POST /v1/:ns/restore  with NDJSON body (Content-Type: application/x-ndjson)
///
/// This is append-only restore — existing facts are not deleted first. Callers
/// that want a clean import should create a fresh namespace first.
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

use polo_core::{
    db::{RecordOpts, ScanQuery},
    fact::{Fact, Namespace},
};

use crate::{ApiError, AppState};

pub async fn dump_namespace(
    State(s): State<AppState>,
    Path(ns): Path<String>,
) -> Result<Response, ApiError> {
    let ns = Namespace::new(&ns);
    let branches = s.db.list_branches(&ns).await?;

    let mut lines = Vec::new();
    for branch in branches {
        let facts = s
            .db
            .scan(ScanQuery {
                namespace: ns.clone(),
                branch: branch.name.clone(),
                include_retracted: true,
                ..Default::default()
            })
            .await?;
        for f in facts {
            if let Ok(line) = serde_json::to_string(&f) {
                lines.push(line);
            }
        }
    }

    let body = lines.join("\n");
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/x-ndjson")],
        body,
    )
        .into_response())
}

#[derive(Debug, Serialize)]
pub struct RestoreResult {
    pub imported: usize,
    pub skipped: usize,
}

pub async fn restore_namespace(
    State(s): State<AppState>,
    Path(ns): Path<String>,
    body: String,
) -> Result<Json<RestoreResult>, ApiError> {
    let ns_name = Namespace::new(&ns);
    let mut imported = 0usize;
    let mut skipped = 0usize;

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let fact: Fact = match serde_json::from_str(line) {
            Ok(f) => f,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };

        let result = s
            .db
            .record(
                ns_name.clone(),
                fact.entity,
                fact.attr,
                fact.value,
                fact.branch,
                RecordOpts {
                    author: fact.author,
                    message: Some("restore".into()),
                    valid_from: Some(fact.valid_from),
                    valid_to: fact.valid_to,
                    // Use original fact ID as idempotency key — safe to replay
                    idempotency_key: Some(fact.id.to_string()),
                    ..Default::default()
                },
            )
            .await;

        match result {
            Ok(r) if r.was_duplicate => skipped += 1,
            Ok(_) => imported += 1,
            Err(_) => skipped += 1,
        }
    }

    Ok(Json(RestoreResult { imported, skipped }))
}
