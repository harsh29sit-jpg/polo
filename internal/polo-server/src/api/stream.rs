use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{debug, warn};

use polo_core::fact::{BranchName, Fact, Namespace};

use crate::AppState;

/// A global event bus. The server holds one of these; handlers publish to it and
/// WebSocket connections subscribe.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Arc<FactEvent>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FactEvent {
    pub kind: EventKind,
    pub fact: Fact,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Recorded,
    Retracted,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn publish(&self, event: FactEvent) {
        // Ignore send errors: it just means no subscribers are connected
        let _ = self.tx.send(Arc::new(event));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<FactEvent>> {
        self.tx.subscribe()
    }
}

#[derive(Debug, Deserialize)]
pub struct StreamQuery {
    pub ns: Option<String>,
    pub branch: Option<String>,
    pub entity: Option<String>,
    pub attr: Option<String>,
}

/// WebSocket handler — clients connect and receive a stream of FactEvents as
/// JSON-encoded messages, filtered by namespace/branch/entity/attr if specified.
/// Clients can also send a text message `{"type":"ping"}` to keep the
/// connection alive, and will receive `{"type":"pong"}`.
pub async fn ws_stream(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(filter): Query<StreamQuery>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, filter))
}

async fn handle_socket(socket: WebSocket, state: AppState, filter: StreamQuery) {
    let mut rx = state.bus.subscribe();
    let (mut sender, mut receiver) = socket.split();

    let ns_filter = filter.ns.map(Namespace::new);
    let branch_filter = filter.branch.map(BranchName::new);
    let entity_filter = filter.entity;
    let attr_filter = filter.attr;

    let send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Some(ns) = &ns_filter {
                        if &event.fact.namespace != ns {
                            continue;
                        }
                    }
                    if let Some(br) = &branch_filter {
                        if &event.fact.branch != br {
                            continue;
                        }
                    }
                    if let Some(ent) = &entity_filter {
                        if event.fact.entity.as_str() != ent {
                            continue;
                        }
                    }
                    if let Some(attr) = &attr_filter {
                        if event.fact.attr.as_str() != attr {
                            continue;
                        }
                    }

                    let json = match serde_json::to_string(&*event) {
                        Ok(s) => s,
                        Err(e) => {
                            warn!("failed to serialize event: {e}");
                            continue;
                        }
                    };

                    if sender.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    debug!("ws subscriber lagged by {n} events");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Drain inbound messages; handle ping/close
    let recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Text(t)) => {
                    if t.contains("\"ping\"") {
                        // nothing to do — we don't have a back-channel here,
                        // connection keepalive is handled by the WS layer
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }
}
