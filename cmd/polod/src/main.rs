use std::sync::Arc;

use anyhow::Context;
use axum::serve;
use clap::Parser;
use tracing::info;

use polo_core::Db;
use polo_server::{build_router, AppState};
use polo_server::api::stream::EventBus;

#[derive(Debug, Parser)]
#[command(name = "polod", about = "polo server daemon", version)]
struct Args {
    /// Path to the SQLite database file
    #[arg(long, env = "POLO_DB", default_value = "polo.db")]
    db: String,

    /// Address to listen on
    #[arg(long, env = "POLO_ADDR", default_value = "0.0.0.0:5432")]
    addr: String,

    /// Bearer token for authentication (disabled if not set)
    #[arg(long, env = "POLO_TOKEN")]
    token: Option<String>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, env = "POLO_LOG", default_value = "info")]
    log: String,

    /// CORS allowed origin ('*' for any)
    #[arg(long, env = "POLO_CORS_ORIGIN")]
    cors_origin: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&args.log)),
        )
        .init();

    info!(db = %args.db, addr = %args.addr, "starting polod");

    let store = polo_store::SqliteStore::open(&args.db)
        .with_context(|| format!("failed to open database at {}", args.db))?;
    let store = Arc::new(store);
    let db = Arc::new(Db::new(store));
    let bus = Arc::new(EventBus::new(1024));

    let state = AppState {
        db,
        token: args.token,
        bus,
    };

    let app = build_router(state, args.cors_origin.as_deref());

    let listener = tokio::net::TcpListener::bind(&args.addr)
        .await
        .with_context(|| format!("failed to bind to {}", args.addr))?;

    info!(addr = %listener.local_addr().unwrap(), "listening");

    serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to install ctrl-c handler");
            info!("shutting down");
        })
        .await
        .context("server error")
}
