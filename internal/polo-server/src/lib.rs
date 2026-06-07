pub mod api;
pub mod auth;
pub mod config;

use std::sync::Arc;

use axum::{
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde_json::json;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use polo_core::{Db, Error};

use api::stream::EventBus;
pub use config::Config;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    pub token: Option<String>,
    pub bus: Arc<EventBus>,
}

pub fn build_router(state: AppState, cors_origin: Option<&str>) -> Router {
    let api = Router::new()
        // namespace management
        .route("/namespaces", get(api::namespaces::list_namespaces))
        .route("/namespaces", post(api::namespaces::create_namespace))
        .route("/namespaces/:ns", get(api::namespaces::get_namespace))
        // facts
        .route("/v1/:ns/facts", post(api::facts::record_fact))
        .route("/v1/:ns/facts/:id", get(api::facts::get_fact))
        .route("/v1/:ns/facts/:id", delete(api::facts::retract_fact))
        // temporal queries
        .route("/v1/:ns/asof", get(api::facts::asof))
        .route("/v1/:ns/effective", get(api::facts::effective))
        .route("/v1/:ns/history", get(api::facts::history))
        .route("/v1/:ns/snapshot/:entity", get(api::facts::snapshot))
        // transactions
        .route("/v1/:ns/transactions", get(api::facts::list_tx))
        .route("/v1/:ns/transactions/:id", get(api::facts::get_tx))
        // branches
        .route("/v1/:ns/branches", get(api::branches::list_branches))
        .route("/v1/:ns/branches", post(api::branches::create_branch))
        .route("/v1/:ns/branches/:name", get(api::branches::get_branch))
        .route("/v1/:ns/branches/:name", delete(api::branches::delete_branch))
        // merge / diff
        .route("/v1/:ns/merge", post(api::branches::merge))
        .route("/v1/:ns/diff", get(api::branches::diff))
        // PQL
        .route("/v1/:ns/query", post(api::query::run_query))
        // entities
        .route("/v1/:ns/entities", get(api::entities::list_entities))
        // dump / restore
        .route("/v1/:ns/dump", get(api::backup::dump_namespace))
        .route("/v1/:ns/restore", post(api::backup::restore_namespace))
        // tags (global across namespaces)
        .route("/v1/tags", get(api::tags::list_tags))
        .route("/v1/tags/:label", get(api::tags::get_tag))
        .route("/v1/tags/:label", axum::routing::put(api::tags::put_tag))
        .route("/v1/tags/:label", delete(api::tags::delete_tag))
        // store stats
        .route("/v1/stats", get(api::stats::get_stats))
        // WebSocket stream
        .route("/v1/stream", get(api::stream::ws_stream))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_token,
        ));

    let mut app = Router::new()
        .route("/healthz", get(api::health::healthz))
        .route("/version", get(api::health::version))
        .merge(api)
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    if let Some(origin) = cors_origin {
        let cors = if origin == "*" {
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
        } else {
            let origin = origin
                .parse::<axum::http::HeaderValue>()
                .expect("invalid CORS origin");
            CorsLayer::new()
                .allow_origin(origin)
                .allow_methods(Any)
                .allow_headers(Any)
        };
        app = app.layer(cors);
    }

    app
}

// Unified error type for API handlers
#[derive(Debug)]
pub struct ApiError(StatusCode, String);

impl ApiError {
    pub fn bad_request(msg: impl ToString) -> Self {
        Self(StatusCode::BAD_REQUEST, msg.to_string())
    }

    pub fn not_found(msg: impl ToString) -> Self {
        Self(StatusCode::NOT_FOUND, msg.to_string())
    }

    pub fn conflict(msg: impl ToString) -> Self {
        Self(StatusCode::CONFLICT, msg.to_string())
    }
}

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        if e.is_not_found() {
            ApiError(StatusCode::NOT_FOUND, e.to_string())
        } else {
            match e {
                Error::Conflict(msg) => ApiError(StatusCode::CONFLICT, msg),
                Error::SchemaViolation { .. } | Error::InvalidTimeRange => {
                    ApiError(StatusCode::UNPROCESSABLE_ENTITY, e.to_string())
                }
                Error::Query(msg) => ApiError(StatusCode::BAD_REQUEST, msg),
                other => {
                    tracing::error!(error = %other, "internal error");
                    ApiError(StatusCode::INTERNAL_SERVER_ERROR, "internal server error".into())
                }
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(json!({ "error": self.1 })),
        )
            .into_response()
    }
}
