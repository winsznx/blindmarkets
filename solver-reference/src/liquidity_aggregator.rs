use crate::intent_monitor::{DecryptedIntent, Fill};
use serde::{Deserialize, Serialize};

pub struct LiquidityAggregator {
    dex_endpoints: Vec<String>,
    client: reqwest::Client,
}

impl LiquidityAggregator {
    pub fn new(dex_endpoints: Vec<String>) -> Self {
        Self {
            dex_endpoints,
            client: reqwest::Client::new(),
        }
    }

    pub async fn find_best_route(&self, intent: &DecryptedIntent) -> Option<Fill> {
        tracing::info!(
            "Finding best route for {} -> {}",
            intent.asset_in,
            intent.asset_out
        );

        let mut best_fill: Option<Fill> = None;

        for endpoint in &self.dex_endpoints {
            let response = self
                .client
                .post(endpoint)
                .json(&QuoteRequest::from(intent))
                .send()
                .await;

            let response = match response {
                Ok(resp) => resp,
                Err(e) => {
                    tracing::warn!("DEX quote request failed: {}", e);
                    continue;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                tracing::warn!("DEX quote returned {}", status);
                continue;
            }

            let quote: QuoteResponse = match response.json().await {
                Ok(value) => value,
                Err(e) => {
                    tracing::warn!("Failed to parse quote response: {}", e);
                    continue;
                }
            };

            let fill = Fill {
                intent_id: intent.intent_id.clone(),
                fill_amount: intent.amount,
                execution_price: quote.execution_price,
                liquidity_source: quote.liquidity_source,
            };

            if best_fill
                .as_ref()
                .map(|current| fill.execution_price > current.execution_price)
                .unwrap_or(true)
            {
                best_fill = Some(fill);
            }
        }

        best_fill
    }

}

impl Default for LiquidityAggregator {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

#[derive(Debug, Serialize)]
struct QuoteRequest {
    asset_in: String,
    asset_out: String,
    amount: u128,
    min_output: u128,
}

impl From<&DecryptedIntent> for QuoteRequest {
    fn from(intent: &DecryptedIntent) -> Self {
        Self {
            asset_in: intent.asset_in.clone(),
            asset_out: intent.asset_out.clone(),
            amount: intent.amount,
            min_output: intent.min_output,
        }
    }
}

#[derive(Debug, Deserialize)]
struct QuoteResponse {
    execution_price: u128,
    liquidity_source: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent_monitor::DecryptedIntent;

    #[tokio::test]
    async fn test_liquidity_aggregator() {
        let aggregator = LiquidityAggregator::new(Vec::new());

        let intent = DecryptedIntent {
            intent_id: "0x123".to_string(),
            user_address: "0xUSER".to_string(),
            asset_in: "0xBTC".to_string(),
            asset_out: "0xUSDC".to_string(),
            amount: 1_000_000,
            amount_commitment: "0xabc".to_string(),
            min_output: 95_000_000,
            max_fee_bps: 50,
            deadline: 9999999999,
            privacy_mode: 0,
            nonce: "0x01".to_string(),
        };

        let fill = aggregator.find_best_route(&intent).await;
        assert!(fill.is_none());
    }
}
