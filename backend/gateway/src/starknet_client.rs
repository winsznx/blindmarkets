use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use crate::config::Config;
use starknet::accounts::{Account, Call, ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::{BlockId, BlockTag, Felt, FunctionCall};
use starknet::providers::jsonrpc::{HttpTransport, JsonRpcClient};
use starknet::providers::Provider;
use starknet::signers::{LocalWallet, SigningKey};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentCommitment {
    pub intent_id: String,
    pub user_address: String,
    pub intent_hash: String,
    pub nonce: String,
    pub asset_in: String,
    pub asset_out: String,
    pub amount_commitment: String,
    pub min_output: String,
    pub max_fee_bps: u16,
    pub deadline: u64,
    pub privacy_mode: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarknetConfig {
    pub rpc_url: String,
    pub chain_id: String,
    pub account_address: String,
    pub private_key: String,
    pub intent_registry_address: String,
}

impl StarknetConfig {
    pub fn from_config(config: &Config) -> Self {
        Self {
            rpc_url: config.starknet.rpc_url.clone(),
            chain_id: config.starknet.chain_id.clone(),
            account_address: config.starknet.account_address.clone(),
            private_key: config.starknet.account_private_key.clone(),
            intent_registry_address: config.starknet.intent_registry_address.clone(),
        }
    }
}

pub struct StarknetClient {
    account: SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    intent_registry_address: Felt,
}

impl StarknetClient {
    pub fn new(config: StarknetConfig) -> Result<Self> {
        let rpc_url = reqwest::Url::parse(&config.rpc_url)
            .map_err(|e| anyhow!("Invalid rpc_url: {}", e))?;
        let provider = JsonRpcClient::new(HttpTransport::new(rpc_url));
        let account_address = parse_felt(&config.account_address, "account_address")?;
        let chain_id = parse_felt(&config.chain_id, "chain_id")?;
        let private_key = parse_felt(&config.private_key, "private_key")?;
        let signer = LocalWallet::from(SigningKey::from_secret_scalar(private_key));
        let account = SingleOwnerAccount::new(
            provider,
            signer,
            account_address,
            chain_id,
            ExecutionEncoding::New,
        );
        let intent_registry_address =
            parse_felt(&config.intent_registry_address, "intent_registry_address")?;

        Ok(Self {
            account,
            intent_registry_address,
        })
    }

    pub async fn commit_intent(
        &self,
        commitment: IntentCommitment,
        user_signature: Vec<String>,
    ) -> Result<String> {
        let mut calldata = Vec::new();
        calldata.push(parse_felt(&commitment.intent_id, "intent_id")?);
        calldata.push(parse_felt(&commitment.user_address, "user_address")?);
        calldata.push(parse_felt(&commitment.intent_hash, "intent_hash")?);
        calldata.push(parse_felt(&commitment.nonce, "nonce")?);
        calldata.push(parse_felt(&commitment.asset_in, "asset_in")?);
        calldata.push(parse_felt(&commitment.asset_out, "asset_out")?);
        calldata.push(parse_felt(&commitment.amount_commitment, "amount_commitment")?);

        let min_output = parse_felt(&commitment.min_output, "min_output")?;
        calldata.push(min_output);
        calldata.push(Felt::ZERO);
        calldata.push(Felt::from(commitment.max_fee_bps as u128));
        calldata.push(Felt::from(commitment.deadline as u128));
        calldata.push(Felt::from(commitment.privacy_mode as u128));

        let signature_fields = parse_signature(&user_signature)?;
        calldata.push(Felt::from(signature_fields.len() as u128));
        calldata.extend(signature_fields);

        let call = Call {
            to: self.intent_registry_address,
            selector: selector_from_name("commit_intent")?,
            calldata,
        };

        let tx = self.account.execute_v1(vec![call]).send().await?;
        Ok(format!("{:#x}", tx.transaction_hash))
    }

    pub async fn get_intent_status(&self, intent_id: &str) -> Result<String> {
        let call = FunctionCall {
            contract_address: self.intent_registry_address,
            entry_point_selector: selector_from_name("get_intent_status")?,
            calldata: vec![parse_felt(intent_id, "intent_id")?],
        };

        let response = self
            .account
            .provider()
            .call(call, BlockId::Tag(BlockTag::Latest))
            .await?;
        let status = response.into_iter().next().unwrap_or(Felt::ZERO);
        Ok(format!("{:#x}", status))
    }

    pub async fn cancel_intent(
        &self,
        intent_id: &str,
        signature: Vec<String>,
    ) -> Result<String> {
        let mut calldata = Vec::new();
        calldata.push(parse_felt(intent_id, "intent_id")?);
        let signature_fields = parse_signature(&signature)?;
        calldata.push(Felt::from(signature_fields.len() as u128));
        calldata.extend(signature_fields);

        let call = Call {
            to: self.intent_registry_address,
            selector: selector_from_name("cancel_intent")?,
            calldata,
        };

        let tx = self.account.execute_v1(vec![call]).send().await?;
        Ok(format!("{:#x}", tx.transaction_hash))
    }
}

fn selector_from_name(name: &str) -> Result<Felt> {
    starknet::core::utils::get_selector_from_name(name)
        .map_err(|e| anyhow!("Failed to derive selector for {}: {}", name, e))
}

fn parse_felt(value: &str, label: &str) -> Result<Felt> {
    if value.starts_with("0x") || value.starts_with("0X") {
        Felt::from_hex(value)
            .map_err(|e| anyhow!("Invalid {} hex: {}", label, e))
    } else {
        Felt::from_dec_str(value)
            .map_err(|e| anyhow!("Invalid {} decimal: {}", label, e))
    }
}

fn parse_signature(signature: &[String]) -> Result<Vec<Felt>> {
    if signature.is_empty() {
        return Err(anyhow!("Signature must not be empty"));
    }
    signature
        .iter()
        .map(|value| parse_felt(value, "signature"))
        .collect::<Result<Vec<_>>>()
}
