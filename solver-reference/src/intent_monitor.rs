use serde::{Deserialize, Serialize};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures::{StreamExt, SinkExt};
use anyhow::Result;
use aes_gcm::{Aes256Gcm, Nonce, aead::{Aead, KeyInit}};
use x25519_dalek::{PublicKey, StaticSecret};
use crate::config::SolverConfig;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedIntent {
    pub intent_id: String,
    pub user_address: String,
    pub ciphertext: String,
    pub commitment: String,
    pub batch_id: String,
    pub received_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecryptedIntent {
    pub intent_id: String,
    pub user_address: String,
    pub asset_in: String,
    pub asset_out: String,
    pub amount: u128,
    pub amount_commitment: String,
    pub min_output: u128,
    pub max_fee_bps: u16,
    pub deadline: u64,
    pub privacy_mode: u8,
    pub nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferPlan {
    pub from: String,
    pub to: String,
    pub asset: String,
    pub amount: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentEnvelope {
    pub batch_id: String,
    pub intent: DecryptedIntent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Solution {
    pub batch_id: String,
    pub solver_address: String,
    pub solution_commitment: String,
    pub estimated_surplus: u128,
    pub solver_fee_bps: u16,
    pub fills: Vec<Fill>,
    pub execution_plan: Vec<TransferPlan>,
    pub proofs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fill {
    pub intent_id: String,
    pub fill_amount: u128,
    pub execution_price: u128,
    pub liquidity_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum GatewayMessage {
    #[serde(rename = "new_intent")]
    NewIntent {
        intent_id: String,
        batch_id: String,
        encrypted_data: String,
        gateway_public_key: String,
        timestamp: i64,
    },
    #[serde(rename = "batch_closed")]
    BatchClosed {
        batch_id: String,
        close_time: i64,
        intent_count: i32,
        auction_deadline: i64,
    },
    #[serde(rename = "connected")]
    Connected {
        message: String,
        timestamp: i64,
    },
}

pub struct IntentMonitor {
    config: SolverConfig,
    intent_sender: mpsc::Sender<IntentEnvelope>,
    batch_sender: mpsc::Sender<String>,
}

impl IntentMonitor {
    pub fn new(
        config: SolverConfig,
        intent_sender: mpsc::Sender<IntentEnvelope>,
        batch_sender: mpsc::Sender<String>,
    ) -> Self {
        Self { config, intent_sender, batch_sender }
    }

    pub async fn connect_websocket(&self) -> Result<()> {
        let ws_url = self.config.gateway_url.replace("http://", "ws://").replace("https://", "wss://");
        let ws_url = format!("{}/v1/solver/ws", ws_url);
        
        tracing::info!("Connecting to gateway WebSocket: {}", ws_url);

        let request = tokio_tungstenite::tungstenite::http::Request::builder()
            .uri(&ws_url)
            .header(self.config.gateway_api_key_header.as_str(), self.config.gateway_api_key.as_str())
            .body(())?;

        let (ws_stream, _) = connect_async(request).await?;
        let (mut write, mut read) = ws_stream.split();
        
        tracing::info!("WebSocket connected successfully");

        if let Ok(public_key) = self.solver_public_key() {
            let hello = serde_json::json!({
                "type": "hello",
                "solver_public_key": public_key
            });
            if let Ok(msg) = serde_json::to_string(&hello) {
                let _ = write.send(Message::Text(msg)).await;
            }
        }

        // Send heartbeat periodically
        let heartbeat_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                let heartbeat = serde_json::json!({
                    "type": "heartbeat",
                    "timestamp": chrono::Utc::now().timestamp()
                });
                if let Ok(msg) = serde_json::to_string(&heartbeat) {
                    if write.send(Message::Text(msg)).await.is_err() {
                        break;
                    }
                }
            }
        });

        // Process incoming messages
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(gateway_msg) = serde_json::from_str::<GatewayMessage>(&text) {
                        self.handle_gateway_message(gateway_msg).await;
                    }
                }
                Ok(Message::Close(_)) => {
                    tracing::info!("WebSocket closed by server");
                    break;
                }
                Err(e) => {
                    tracing::error!("WebSocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }

        heartbeat_task.abort();
        Ok(())
    }

    async fn handle_gateway_message(&self, msg: GatewayMessage) {
        match msg {
            GatewayMessage::NewIntent { intent_id, batch_id, encrypted_data, gateway_public_key, .. } => {
                tracing::info!("Received new intent: {} in batch: {}", intent_id, batch_id);
                // Decrypt and process intent
                if let Ok(decrypted) = self.decrypt_intent(&encrypted_data, &gateway_public_key) {
                    let envelope = IntentEnvelope {
                        batch_id,
                        intent: decrypted,
                    };
                    if self.intent_sender.send(envelope).await.is_err() {
                        tracing::error!("Failed to forward decrypted intent");
                    }
                }
            }
            GatewayMessage::BatchClosed { batch_id, intent_count, .. } => {
                tracing::info!("Batch {} closed with {} intents", batch_id, intent_count);
                if self.batch_sender.send(batch_id).await.is_err() {
                    tracing::error!("Failed to forward batch close signal");
                }
            }
            GatewayMessage::Connected { message, .. } => {
                tracing::info!("Gateway connection confirmed: {}", message);
            }
        }
    }

    fn decrypt_intent(&self, encrypted_data: &str, gateway_public_key: &str) -> Result<DecryptedIntent> {
        let ciphertext = hex::decode(encrypted_data.trim_start_matches("0x"))?;
        if ciphertext.len() < 12 {
            return Err(anyhow::anyhow!("Ciphertext too short"));
        }

        let mut private_key_bytes = [0u8; 32];
        let solver_key = hex::decode(self.config.solver_private_key.trim_start_matches("0x"))?;
        if solver_key.len() != 32 {
            return Err(anyhow::anyhow!("SOLVER_PRIVATE_KEY must be 32 bytes"));
        }
        private_key_bytes.copy_from_slice(&solver_key);

        let mut client_key_bytes = [0u8; 32];
        let client_key = hex::decode(gateway_public_key.trim_start_matches("0x"))?;
        if client_key.len() != 32 {
            return Err(anyhow::anyhow!("gateway_public_key must be 32 bytes"));
        }
        client_key_bytes.copy_from_slice(&client_key);

        let private_key = StaticSecret::from(private_key_bytes);
        let public_key = PublicKey::from(client_key_bytes);
        let shared_secret = private_key.diffie_hellman(&public_key).to_bytes();

        let cipher = Aes256Gcm::new_from_slice(&shared_secret)?;
        let nonce = Nonce::from_slice(&ciphertext[..12]);
        let plaintext = cipher.decrypt(nonce, &ciphertext[12..]).map_err(|e| anyhow::anyhow!("Decryption failed: {:?}", e))?;

        let decrypted: DecryptedIntent = serde_json::from_slice(&plaintext)?;
        Ok(decrypted)
    }

    fn solver_public_key(&self) -> Result<String> {
        let mut private_key_bytes = [0u8; 32];
        let solver_key = hex::decode(self.config.solver_private_key.trim_start_matches("0x"))?;
        if solver_key.len() != 32 {
            return Err(anyhow::anyhow!("SOLVER_PRIVATE_KEY must be 32 bytes"));
        }
        private_key_bytes.copy_from_slice(&solver_key);
        let private_key = StaticSecret::from(private_key_bytes);
        let public_key = PublicKey::from(&private_key).to_bytes();
        Ok(format!("0x{}", hex::encode(public_key)))
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SolverConfig;

    #[test]
    fn test_intent_monitor_creation() {
        let config = SolverConfig {
            solver_address: "0xSOLVER".to_string(),
            gateway_url: "http://localhost:3000".to_string(),
            gateway_api_key_header: "X-API-Key".to_string(),
            gateway_api_key: "test".to_string(),
            solver_private_key: "0x1111111111111111111111111111111111111111111111111111111111111111".to_string(),
            solver_account_address: "0xACCOUNT".to_string(),
            solver_account_private_key: "0x2222222222222222222222222222222222222222222222222222222222222222".to_string(),
            chain_id: "0x534e5f4d41494e".to_string(),
            batch_auction_address: "0xBATCH".to_string(),
            batch_settlement_address: "0xSETTLE".to_string(),
            solver_bond_address: "0xBOND".to_string(),
            rpc_url: "http://localhost:5050".to_string(),
            solver_fee_bps: 50,
            solver_bond_proof: "0xPROOF".to_string(),
            dex_quote_endpoints: vec!["http://localhost:8080/quote".to_string()],
            initial_inventory: vec![("0xUSDC".to_string(), 1_000_000)],
            proof_service_url: None,
            proof_service_api_key: None,
        };
        let (intent_tx, _intent_rx) = mpsc::channel(1);
        let (batch_tx, _batch_rx) = mpsc::channel(1);
        let monitor = IntentMonitor::new(config.clone(), intent_tx, batch_tx);
        assert_eq!(monitor.config.gateway_url, "http://localhost:3000");
    }

    #[test]
    fn test_message_deserialization() {
        let json = r#"{"type":"new_intent","intent_id":"123","batch_id":"456","encrypted_data":"0xabc","gateway_public_key":"0xaaaabbbbccccddddeeeeffff0000111122223333444455556666777788889999","timestamp":1234567890}"#;
        let msg: Result<GatewayMessage, _> = serde_json::from_str(json);
        assert!(msg.is_ok());
    }
}
