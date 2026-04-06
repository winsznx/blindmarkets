use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    Extension,
    http::{HeaderMap, StatusCode},
};
use futures::{sink::SinkExt, stream::StreamExt};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn, error};

use crate::config::Config;
use crate::crypto;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentNotification {
    pub intent_id: String,
    pub batch_id: String,
    pub encrypted_data: String,
    pub gateway_public_key: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchNotification {
    pub batch_id: String,
    pub close_time: i64,
    pub intent_count: i32,
    pub auction_deadline: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SolverMessage {
    #[serde(rename = "new_intent")]
    NewIntent(IntentNotification),
    #[serde(rename = "batch_closed")]
    BatchClosed(BatchNotification),
    #[serde(rename = "ping")]
    Ping { timestamp: i64 },
}

#[derive(Debug, Clone)]
pub enum SolverBroadcast {
    NewIntent {
        intent_id: String,
        batch_id: String,
        plaintext: Vec<u8>,
    },
    BatchClosed(BatchNotification),
}

pub type SolverBroadcaster = broadcast::Sender<SolverBroadcast>;

pub fn create_broadcaster(capacity: usize) -> SolverBroadcaster {
    let (tx, _rx) = broadcast::channel(capacity);
    tx
}

pub async fn solver_websocket_handler(
    ws: WebSocketUpgrade,
    State(pool): State<PgPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(broadcaster): Extension<Arc<SolverBroadcaster>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = require_api_key(&headers, &config) {
        return status.into_response();
    }
    ws.on_upgrade(move |socket| handle_solver_socket(socket, pool, broadcaster, config))
}

async fn handle_solver_socket(
    socket: WebSocket,
    pool: PgPool,
    broadcaster: Arc<SolverBroadcaster>,
    config: Arc<Config>,
) {
    let (mut sender, mut receiver) = socket.split();
    let solver_public_key = Arc::new(tokio::sync::RwLock::new(None::<String>));

    // Channel for replaying missed intents: recv_task produces, send_task consumes.
    let (replay_tx, mut replay_rx) = tokio::sync::mpsc::channel::<String>(256);

    let mut rx = broadcaster.subscribe();

    info!("New solver WebSocket connection established");

    // Send initial connection confirmation
    let welcome = serde_json::json!({
        "type": "connected",
        "message": "Connected to Blind BTC Intent Markets",
        "timestamp": chrono::Utc::now().timestamp()
    });

    if let Ok(msg) = serde_json::to_string(&welcome) {
        let _ = sender.send(Message::Text(msg)).await;
    }

    // Spawn task to send broadcasts to this client
    let send_solver_key = solver_public_key.clone();
    let send_config = config.clone();
    let mut send_task = tokio::spawn(async move {
        let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(15));
        ping_interval.tick().await; // consume the immediate first tick

        loop {
            tokio::select! {
                // Replay: pre-built JSON from recv_task (catch-up on connect)
                Some(json) = replay_rx.recv() => {
                    if sender.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
                recv_result = rx.recv() => {
                    match recv_result {
                        Ok(msg) => match msg {
                            SolverBroadcast::NewIntent { intent_id, batch_id, plaintext } => {
                                let solver_key = send_solver_key.read().await.clone();
                                if let Some(key) = solver_key {
                                    match crypto::encrypt_for_solver(&plaintext, &send_config.security.gateway_private_key, &key) {
                                        Ok(payload) => {
                                            let outbound = SolverMessage::NewIntent(IntentNotification {
                                                intent_id: intent_id.clone(),
                                                batch_id: batch_id.clone(),
                                                encrypted_data: payload.ciphertext_hex,
                                                gateway_public_key: payload.sender_public_key_hex,
                                                timestamp: chrono::Utc::now().timestamp(),
                                            });
                                            if let Ok(json) = serde_json::to_string(&outbound) {
                                                if sender.send(Message::Text(json)).await.is_err() {
                                                    break;
                                                }
                                                info!("Sent new_intent {} (batch {}) to solver", intent_id, batch_id);
                                            }
                                        }
                                        Err(e) => {
                                            error!("Failed to encrypt intent for solver: {}", e);
                                        }
                                    }
                                } else {
                                    warn!("Solver public key not set; skipping intent broadcast");
                                }
                            }
                            SolverBroadcast::BatchClosed(notification) => {
                                let outbound = SolverMessage::BatchClosed(notification);
                                if let Ok(json) = serde_json::to_string(&outbound) {
                                    if sender.send(Message::Text(json)).await.is_err() {
                                        break;
                                    }
                                }
                            }
                        },
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            warn!("Solver broadcast receiver lagged by {} messages", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }
                _ = ping_interval.tick() => {
                    let outbound = SolverMessage::Ping { timestamp: chrono::Utc::now().timestamp() };
                    if let Ok(json) = serde_json::to_string(&outbound) {
                        if sender.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    });

    // Spawn task to receive messages from client
    let recv_solver_key = solver_public_key.clone();
    let recv_config = config.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                        handle_solver_message(parsed, &pool, &recv_solver_key, &recv_config, &replay_tx).await;
                    }
                }
                Message::Close(_) => {
                    info!("Solver WebSocket closed");
                    break;
                }
                Message::Ping(data) => {
                    info!("Received ping: {:?}", data);
                }
                _ => {}
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = (&mut send_task) => {
            recv_task.abort();
        }
        _ = (&mut recv_task) => {
            send_task.abort();
        }
    }

    info!("Solver WebSocket connection closed");
}

async fn handle_solver_message(
    msg: serde_json::Value,
    pool: &PgPool,
    solver_public_key: &Arc<tokio::sync::RwLock<Option<String>>>,
    config: &Config,
    replay_tx: &tokio::sync::mpsc::Sender<String>,
) {
    let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match msg_type {
        "hello" => {
            if let Some(key) = msg.get("solver_public_key").and_then(|v| v.as_str()) {
                if key.starts_with("0x") && key.len() == 66 {
                    {
                        let mut lock = solver_public_key.write().await;
                        *lock = Some(key.to_string());
                    }
                    info!("Solver public key registered");
                    replay_pending_intents(pool, config, key, replay_tx).await;
                } else {
                    warn!("Invalid solver_public_key format");
                }
            }
        }
        "subscribe_batch" => {
            if let Some(batch_id) = msg.get("batch_id").and_then(|v| v.as_str()) {
                info!("Solver subscribed to batch: {}", batch_id);
            }
        }
        "heartbeat" => {}
        _ => {
            warn!("Unknown message type: {}", msg_type);
        }
    }
}

async fn replay_pending_intents(
    pool: &PgPool,
    config: &Config,
    solver_key: &str,
    tx: &tokio::sync::mpsc::Sender<String>,
) {
    let rows = sqlx::query!(
        r#"
        SELECT intent_id, batch_id, ciphertext, encrypted_session_key, client_public_key
        FROM intents
        WHERE status IN ('PENDING', 'AUCTION')
          AND submission_mode = 'GATEWAY'
          AND encrypted_session_key IS NOT NULL
        ORDER BY created_at ASC
        "#
    )
    .fetch_all(pool)
    .await;

    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to query pending intents for replay: {}", e);
            return;
        }
    };

    info!("Replaying {} pending/auction intents to newly connected solver", rows.len());

    for row in rows {
        let Some(enc_session_key) = row.encrypted_session_key else {
            continue;
        };
        let client_pub_key = row.client_public_key;

        let plaintext = match crate::crypto::decrypt_from_client(
            &row.ciphertext,
            &enc_session_key,
            &client_pub_key,
            &config.security.gateway_private_key,
        ) {
            Ok(p) => p,
            Err(e) => {
                warn!("Failed to decrypt intent {} for replay: {}", row.intent_id, e);
                continue;
            }
        };

        let payload = match crate::crypto::encrypt_for_solver(
            &plaintext,
            &config.security.gateway_private_key,
            solver_key,
        ) {
            Ok(p) => p,
            Err(e) => {
                warn!("Failed to re-encrypt intent {} for solver: {}", row.intent_id, e);
                continue;
            }
        };

        let outbound = SolverMessage::NewIntent(IntentNotification {
            intent_id: row.intent_id.clone(),
            batch_id: row.batch_id,
            encrypted_data: payload.ciphertext_hex,
            gateway_public_key: payload.sender_public_key_hex,
            timestamp: chrono::Utc::now().timestamp(),
        });

        if let Ok(json) = serde_json::to_string(&outbound) {
            if tx.send(json).await.is_err() {
                warn!("Replay channel closed before intent {} was sent", row.intent_id);
                return;
            }
        }
    }
}

// Helper function to broadcast new intent to all connected solvers
pub async fn broadcast_new_intent(
    broadcaster: &SolverBroadcaster,
    intent_id: String,
    batch_id: String,
    plaintext: Vec<u8>,
) {
    if let Err(e) = broadcaster.send(SolverBroadcast::NewIntent {
        intent_id,
        batch_id,
        plaintext,
    }) {
        error!("Failed to broadcast intent: {}", e);
    }
}

// Helper function to broadcast batch closure
pub async fn broadcast_batch_closed(
    broadcaster: &SolverBroadcaster,
    batch_id: String,
    close_time: i64,
    intent_count: i32,
    auction_deadline: i64,
) {
    let notification = BatchNotification {
        batch_id,
        close_time,
        intent_count,
        auction_deadline,
    };

    if let Err(e) = broadcaster.send(SolverBroadcast::BatchClosed(notification)) {
        error!("Failed to broadcast batch closure: {}", e);
    }
}

fn require_api_key(headers: &HeaderMap, config: &Config) -> Result<(), StatusCode> {
    let header_name = config.security.api_key_header.as_str();
    let provided = headers.get(header_name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");

    if provided.is_empty() {
        warn!("Missing API key header: {}", header_name);
        return Err(StatusCode::UNAUTHORIZED);
    }

    if provided != config.security.api_key {
        warn!("Invalid API key");
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_serialization() {
        let msg = SolverMessage::NewIntent(IntentNotification {
            intent_id: "intent_123".to_string(),
            batch_id: "456".to_string(),
            encrypted_data: "0xencrypted".to_string(),
            gateway_public_key: "0xgateway".to_string(),
            timestamp: 1234567890,
        });

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("new_intent"));
        assert!(json.contains("intent_123"));
    }

    #[test]
    fn test_batch_notification() {
        let msg = SolverMessage::BatchClosed(BatchNotification {
            batch_id: "789".to_string(),
            close_time: 1234567890,
            intent_count: 42,
            auction_deadline: 1234567920,
        });

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("batch_closed"));
        assert!(json.contains("batch_789"));
    }
}
