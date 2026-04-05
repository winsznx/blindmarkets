use crate::intent_monitor::{DecryptedIntent, Solution, Fill, TransferPlan};
use crate::starknet_client::{StarknetClient, StarknetConfig};
use anyhow::anyhow;
use starknet_crypto::{Felt, pedersen_hash};
use std::collections::HashMap;

pub struct SolutionBuilder {
    solver_address: String,
    solver_fee_bps: u16,
    bond_proof: String,
    client: StarknetClient,
}

impl SolutionBuilder {
    pub fn new(
        solver_address: String,
        solver_fee_bps: u16,
        bond_proof: String,
        starknet: StarknetConfig,
    ) -> Result<Self, anyhow::Error> {
        Ok(Self {
            solver_address,
            solver_fee_bps,
            bond_proof,
            client: StarknetClient::new(starknet)?,
        })
    }

    pub fn build_solution(
        &self,
        batch_id: String,
        fills: Vec<Fill>,
        execution_plan: Vec<TransferPlan>,
        proofs: Vec<String>,
    ) -> Result<Solution, anyhow::Error> {
        let estimated_surplus = self.calculate_total_surplus(&fills);
        let solution_commitment = self.compute_settlement_hash(&batch_id, &execution_plan)?;

        Ok(Solution {
            batch_id,
            solver_address: self.solver_address.clone(),
            solution_commitment,
            estimated_surplus,
            solver_fee_bps: self.solver_fee_bps,
            fills,
            execution_plan,
            proofs,
        })
    }

    fn calculate_total_surplus(&self, fills: &[Fill]) -> u128 {
        fills.iter()
            .map(|fill| {
                // Surplus = (execution_price - min_acceptable_price) * amount
                // Simplified: use 1% of fill amount as surplus
                fill.fill_amount / 100
            })
            .sum()
    }

    pub fn build_execution_plan(
        &self,
        intents: &[DecryptedIntent],
        fills: &[Fill],
    ) -> Result<Vec<TransferPlan>, anyhow::Error> {
        let fill_map = fills
            .iter()
            .map(|fill| (fill.intent_id.clone(), fill))
            .collect::<HashMap<_, _>>();

        let mut plan = Vec::new();
        for intent in intents {
            let fill = fill_map
                .get(&intent.intent_id)
                .ok_or_else(|| anyhow!("Missing fill for intent {}", intent.intent_id))?;
            let output_amount = compute_output_amount(fill, intent.min_output)?;
            plan.push(TransferPlan {
                from: intent.user_address.clone(),
                to: self.solver_address.clone(),
                asset: intent.asset_in.clone(),
                amount: intent.amount,
            });
            plan.push(TransferPlan {
                from: self.solver_address.clone(),
                to: intent.user_address.clone(),
                asset: intent.asset_out.clone(),
                amount: output_amount,
            });
        }

        Ok(plan)
    }

    pub fn compute_settlement_hash(
        &self,
        batch_id: &str,
        execution_plan: &[TransferPlan],
    ) -> Result<String, anyhow::Error> {
        let mut hash = field_from_hex(batch_id)?;

        for transfer in execution_plan {
            let from = field_from_hex(&transfer.from)?;
            let to = field_from_hex(&transfer.to)?;
            let asset = field_from_hex(&transfer.asset)?;
            let amount_low = Felt::from(transfer.amount);
            let amount_high = Felt::ZERO;
            let amount_hash = pedersen_hash(&amount_low, &amount_high);
            let transfer_hash = pedersen_hash(
                &pedersen_hash(&from, &to),
                &pedersen_hash(&asset, &amount_hash),
            );
            hash = pedersen_hash(&hash, &transfer_hash);
        }

        Ok(format!("{hash:#x}"))
    }

    pub async fn submit_solution(&self, solution: &Solution) -> Result<String, anyhow::Error> {
        tracing::info!(
            "Submitting solution for batch {} with {} fills",
            solution.batch_id,
            solution.fills.len()
        );

        self.client.submit_solution(
            &solution.batch_id,
            &solution.solution_commitment,
            solution.estimated_surplus,
            solution.solver_fee_bps,
            &self.bond_proof,
        ).await
    }

    pub async fn submit_settlement(&self, solution: &Solution) -> Result<String, anyhow::Error> {
        self.client.settle_batch(
            &solution.batch_id,
            &solution.solver_address,
            &solution.execution_plan,
            &solution.proofs,
        ).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_solution_builder() {
        let builder = SolutionBuilder::new(
            "0xSOLVER".to_string(),
            30,
            "0x0".to_string(),
            StarknetConfig {
                rpc_url: "http://localhost:5050".to_string(),
                chain_id: "0x534e5f4d41494e".to_string(),
                account_address: "0x1".to_string(),
                private_key: "0x2".to_string(),
                batch_auction_address: "0x123".to_string(),
                batch_settlement_address: "0x456".to_string(),
                solver_bond_address: "0x789".to_string(),
            },
        ).unwrap();

        let fills = vec![
            Fill {
                intent_id: "0x123".to_string(),
                fill_amount: 1_000_000,
                execution_price: 95_000,
                liquidity_source: "internal".to_string(),
            },
        ];

        let plan = vec![TransferPlan {
            from: "0x1".to_string(),
            to: "0x2".to_string(),
            asset: "0x3".to_string(),
            amount: 1_000_000,
        }];
        let solution = builder.build_solution("0x1".to_string(), fills, plan, vec![]).unwrap();

        assert_eq!(solution.batch_id, "0x1");
        assert_eq!(solution.solver_fee_bps, 30);
        assert!(solution.solution_commitment.starts_with("0x"));
    }
}

fn compute_output_amount(fill: &Fill, min_output: u128) -> Result<u128, anyhow::Error> {
    let numerator = fill.fill_amount
        .checked_mul(fill.execution_price)
        .ok_or_else(|| anyhow!("Output amount overflow"))?;
    let output = numerator / 1_000_000;
    Ok(std::cmp::max(output, min_output))
}

fn field_from_hex(value: &str) -> Result<Felt, anyhow::Error> {
    if value.starts_with("0x") || value.starts_with("0X") {
        Felt::from_hex(value)
            .map_err(|e| anyhow!("Invalid hex: {}", e))
    } else {
        Felt::from_dec_str(value)
            .map_err(|e| anyhow!("Invalid decimal: {}", e))
    }
}
