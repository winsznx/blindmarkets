use anyhow::{anyhow, Result};
use serde::Serialize;
use starknet::accounts::{Account, SingleOwnerAccount};
use starknet::accounts::Call;
use starknet::core::types::{BlockId, BlockTag, FieldElement, FunctionCall};
use starknet::providers::jsonrpc::{HttpTransport, JsonRpcClient};
use starknet::providers::Provider;
use starknet::signers::{LocalWallet, SigningKey};
use std::sync::Arc;

#[derive(Clone, Serialize)]
pub struct StarknetConfig {
    pub rpc_url: String,
    pub chain_id: String,
    pub account_address: String,
    pub private_key: String,
    pub batch_auction_address: String,
    pub batch_settlement_address: String,
    pub solver_bond_address: String,
}

#[derive(Clone)]
pub struct StarknetClient {
    account: SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>,
    provider: Arc<JsonRpcClient<HttpTransport>>,
    batch_auction_address: FieldElement,
    batch_settlement_address: FieldElement,
    solver_bond_address: FieldElement,
}

impl StarknetClient {
    pub fn new(config: StarknetConfig) -> Result<Self> {
        let provider = Arc::new(JsonRpcClient::new(HttpTransport::new(config.rpc_url)));
        let account_address = parse_field_element(&config.account_address, "account_address")?;
        let chain_id = parse_field_element(&config.chain_id, "chain_id")?;
        let private_key = parse_field_element(&config.private_key, "private_key")?;
        let signer = LocalWallet::from(SigningKey::from_secret_scalar(private_key));
        let account = SingleOwnerAccount::new(Arc::clone(&provider), signer, account_address, chain_id);
        let batch_auction_address =
            parse_field_element(&config.batch_auction_address, "batch_auction_address")?;
        let batch_settlement_address =
            parse_field_element(&config.batch_settlement_address, "batch_settlement_address")?;
        let solver_bond_address =
            parse_field_element(&config.solver_bond_address, "solver_bond_address")?;

        Ok(Self {
            account,
            provider,
            batch_auction_address,
            batch_settlement_address,
            solver_bond_address,
        })
    }

    pub async fn submit_solution(
        &self,
        batch_id: &str,
        solution_commitment: &str,
        estimated_surplus: u128,
        solver_fee_bps: u16,
        bond_proof: &str,
    ) -> Result<String> {
        let mut calldata = Vec::new();
        calldata.push(parse_field_element(batch_id, "batch_id")?);
        calldata.push(parse_field_element(solution_commitment, "solution_commitment")?);
        calldata.push(FieldElement::from(estimated_surplus));
        calldata.push(FieldElement::ZERO);
        calldata.push(FieldElement::from(solver_fee_bps as u128));
        calldata.push(parse_field_element(bond_proof, "bond_proof")?);

        let call = Call {
            to: self.batch_auction_address,
            selector: selector_from_name("submit_solution")?,
            calldata,
        };

        let tx = self.account.execute(vec![call]).send().await?;
        Ok(format!("{:#x}", tx.transaction_hash))
    }

    pub async fn settle_batch(
        &self,
        batch_id: &str,
        solver_address: &str,
        execution_plan: &[super::intent_monitor::TransferPlan],
        proofs: &[String],
    ) -> Result<String> {
        let mut calldata = Vec::new();
        calldata.push(parse_field_element(batch_id, "batch_id")?);
        calldata.push(parse_field_element(solver_address, "solver")?);

        calldata.push(FieldElement::from(execution_plan.len() as u128));
        for transfer in execution_plan {
            calldata.push(parse_field_element(&transfer.from, "transfer.from")?);
            calldata.push(parse_field_element(&transfer.to, "transfer.to")?);
            calldata.push(parse_field_element(&transfer.asset, "transfer.asset")?);
            calldata.push(FieldElement::from(transfer.amount));
            calldata.push(FieldElement::ZERO);
        }

        calldata.push(FieldElement::from(proofs.len() as u128));
        for proof in proofs {
            calldata.push(parse_field_element(proof, "proof")?);
        }

        let call = Call {
            to: self.batch_settlement_address,
            selector: selector_from_name("settle_batch")?,
            calldata,
        };

        let tx = self.account.execute(vec![call]).send().await?;
        Ok(format!("{:#x}", tx.transaction_hash))
    }

    pub async fn deposit_bond(&self, amount: u128) -> Result<String> {
        let calldata = vec![
            FieldElement::from(amount),
            FieldElement::ZERO,
        ];
        let call = Call {
            to: self.solver_bond_address,
            selector: selector_from_name("deposit_bond")?,
            calldata,
        };
        let tx = self.account.execute(vec![call]).send().await?;
        Ok(format!("{:#x}", tx.transaction_hash))
    }

    pub async fn request_withdrawal(&self, amount: u128) -> Result<String> {
        let calldata = vec![
            FieldElement::from(amount),
            FieldElement::ZERO,
        ];
        let call = Call {
            to: self.solver_bond_address,
            selector: selector_from_name("request_withdrawal")?,
            calldata,
        };
        let tx = self.account.execute(vec![call]).send().await?;
        Ok(format!("{:#x}", tx.transaction_hash))
    }

    pub async fn withdraw(&self, amount: u128) -> Result<String> {
        let calldata = vec![
            FieldElement::from(amount),
            FieldElement::ZERO,
        ];
        let call = Call {
            to: self.solver_bond_address,
            selector: selector_from_name("withdraw")?,
            calldata,
        };
        let tx = self.account.execute(vec![call]).send().await?;
        Ok(format!("{:#x}", tx.transaction_hash))
    }

    pub async fn get_solver_info(&self, solver_address: &str) -> Result<SolverInfo> {
        let call = FunctionCall {
            contract_address: self.solver_bond_address,
            entry_point_selector: selector_from_name("get_solver_info")?,
            calldata: vec![parse_field_element(solver_address, "solver_address")?],
        };

        let response = self.provider.call(call, BlockId::Tag(BlockTag::Latest)).await?;
        if response.len() < 12 {
            return Err(anyhow!("Invalid solver info response length"));
        }

        Ok(SolverInfo {
            solver: format!("{:#x}", response[0]),
            bond_amount: format_u256(response[1], response[2]),
            locked: response[3] != FieldElement::ZERO,
            successful_settlements: response[4].to_u128().unwrap_or(0) as u32,
            failed_settlements: response[5].to_u128().unwrap_or(0) as u32,
            slash_count_30d: response[6].to_u128().unwrap_or(0) as u8,
            last_slash_timestamp: response[7].to_u128().unwrap_or(0) as u64,
            withdrawal_request_time: response[8].to_u128().unwrap_or(0) as u64,
            withdrawal_request_amount: format_u256(response[9], response[10]),
            blacklisted: response[11] != FieldElement::ZERO,
        })
    }
}

#[derive(Clone, Debug)]
pub struct SolverInfo {
    pub solver: String,
    pub bond_amount: String,
    pub locked: bool,
    pub successful_settlements: u32,
    pub failed_settlements: u32,
    pub slash_count_30d: u8,
    pub last_slash_timestamp: u64,
    pub withdrawal_request_time: u64,
    pub withdrawal_request_amount: String,
    pub blacklisted: bool,
}

fn selector_from_name(name: &str) -> Result<FieldElement> {
    starknet::core::utils::get_selector_from_name(name)
        .map_err(|e| anyhow!("Failed to derive selector for {}: {}", name, e))
}

fn parse_field_element(value: &str, label: &str) -> Result<FieldElement> {
    if let Some(stripped) = value.strip_prefix("0x") {
        FieldElement::from_hex_be(stripped)
            .map_err(|e| anyhow!("Invalid {} hex: {}", label, e))
    } else {
        FieldElement::from_dec_str(value)
            .map_err(|e| anyhow!("Invalid {} decimal: {}", label, e))
    }
}

fn format_u256(low: FieldElement, high: FieldElement) -> String {
    let low_u128 = low.to_u128().unwrap_or(0);
    let high_u128 = high.to_u128().unwrap_or(0);
    if high_u128 == 0 {
        format!("{:#x}", low_u128)
    } else {
        format!("0x{:x}{:032x}", high_u128, low_u128)
    }
}
