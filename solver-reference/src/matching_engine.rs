use crate::intent_monitor::{DecryptedIntent, Fill};

pub struct MatchingEngine {
    internal_inventory: std::collections::HashMap<String, u128>,
    accept_all: bool,
}

impl MatchingEngine {
    pub fn new() -> Self {
        Self {
            internal_inventory: std::collections::HashMap::new(),
            accept_all: false,
        }
    }

    pub fn with_accept_all(mut self) -> Self {
        self.accept_all = true;
        self
    }

    pub fn can_fill_internally(&self, intent: &DecryptedIntent) -> bool {
        // Check if solver has internal inventory to fill
        let available = self.internal_inventory
            .get(&intent.asset_out)
            .copied()
            .unwrap_or(0);
        
        available >= intent.min_output
    }

    pub fn create_internal_fill(&self, intent: &DecryptedIntent) -> Option<Fill> {
        if self.accept_all {
            // Demo/test mode: fill every intent at min_output price.
            let execution_price = if intent.amount > 0 {
                (intent.min_output * 1_000_000) / intent.amount
            } else {
                1_000_000
            };
            return Some(Fill {
                intent_id: intent.intent_id.clone(),
                fill_amount: intent.amount,
                execution_price,
                liquidity_source: "accept_all".to_string(),
            });
        }

        if !self.can_fill_internally(intent) {
            return None;
        }

        let execution_price = if intent.amount > 0 {
            (intent.min_output * 1_000_000) / intent.amount
        } else {
            1_000_000
        };

        Some(Fill {
            intent_id: intent.intent_id.clone(),
            fill_amount: intent.amount,
            execution_price,
            liquidity_source: "internal_inventory".to_string(),
        })
    }

    pub fn add_inventory(&mut self, asset: String, amount: u128) {
        *self.internal_inventory.entry(asset).or_insert(0) += amount;
    }
}

impl Default for MatchingEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent_monitor::DecryptedIntent;

    #[test]
    fn test_matching_engine() {
        let mut engine = MatchingEngine::new();
        
        engine.add_inventory("0xUSDC".to_string(), 1_000_000_000);

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

        assert!(engine.can_fill_internally(&intent));
        
        let fill = engine.create_internal_fill(&intent).unwrap();
        assert_eq!(fill.intent_id, "0x123");
        assert_eq!(fill.fill_amount, 1_000_000);
    }
}
