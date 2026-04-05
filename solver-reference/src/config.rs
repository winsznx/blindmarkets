use serde::Deserialize;
use std::env;

#[derive(Debug, Clone, Deserialize)]
pub struct SolverConfig {
    pub solver_address: String,
    pub gateway_url: String,
    pub gateway_api_key_header: String,
    pub gateway_api_key: String,
    pub solver_private_key: String,
    pub solver_account_address: String,
    pub solver_account_private_key: String,
    pub chain_id: String,
    pub batch_auction_address: String,
    pub batch_settlement_address: String,
    pub solver_bond_address: String,
    pub rpc_url: String,
    pub solver_fee_bps: u16,
    pub solver_bond_proof: String,
    pub dex_quote_endpoints: Vec<String>,
    pub initial_inventory: Vec<(String, u128)>,
    pub proof_service_url: Option<String>,
    pub proof_service_api_key: Option<String>,
    /// When true, solver fills every intent at min_output without needing
    /// real inventory or DEX quotes. For testing / demo only.
    pub accept_all_intents: bool,
}

impl SolverConfig {
    pub fn from_env() -> Result<Self, String> {
        let solver_address = env::var("SOLVER_ADDRESS")
            .map_err(|_| "SOLVER_ADDRESS environment variable not set".to_string())?;
        let gateway_url = env::var("GATEWAY_URL")
            .map_err(|_| "GATEWAY_URL environment variable not set".to_string())?;
        let gateway_api_key_header = env::var("GATEWAY_API_KEY_HEADER")
            .map_err(|_| "GATEWAY_API_KEY_HEADER environment variable not set".to_string())?;
        let gateway_api_key = env::var("GATEWAY_API_KEY")
            .map_err(|_| "GATEWAY_API_KEY environment variable not set".to_string())?;
        let solver_private_key = env::var("SOLVER_PRIVATE_KEY")
            .map_err(|_| "SOLVER_PRIVATE_KEY environment variable not set".to_string())?;
        let solver_account_address = env::var("SOLVER_ACCOUNT_ADDRESS")
            .map_err(|_| "SOLVER_ACCOUNT_ADDRESS environment variable not set".to_string())?;
        let solver_account_private_key = env::var("SOLVER_ACCOUNT_PRIVATE_KEY")
            .map_err(|_| "SOLVER_ACCOUNT_PRIVATE_KEY environment variable not set".to_string())?;
        let chain_id = env::var("STARKNET_CHAIN_ID")
            .map_err(|_| "STARKNET_CHAIN_ID environment variable not set".to_string())?;
        let batch_auction_address = env::var("BATCH_AUCTION_ADDRESS")
            .map_err(|_| "BATCH_AUCTION_ADDRESS environment variable not set".to_string())?;
        let batch_settlement_address = env::var("BATCH_SETTLEMENT_ADDRESS")
            .map_err(|_| "BATCH_SETTLEMENT_ADDRESS environment variable not set".to_string())?;
        let solver_bond_address = env::var("SOLVER_BOND_ADDRESS")
            .map_err(|_| "SOLVER_BOND_ADDRESS environment variable not set".to_string())?;
        let rpc_url = env::var("STARKNET_RPC_URL")
            .map_err(|_| "STARKNET_RPC_URL environment variable not set".to_string())?;
        let solver_fee_bps = env::var("SOLVER_FEE_BPS")
            .map_err(|_| "SOLVER_FEE_BPS environment variable not set".to_string())?
            .parse()
            .map_err(|e| format!("Invalid SOLVER_FEE_BPS: {}", e))?;
        let solver_bond_proof = env::var("SOLVER_BOND_PROOF")
            .map_err(|_| "SOLVER_BOND_PROOF environment variable not set".to_string())?;
        let dex_quote_endpoints = env::var("DEX_QUOTE_ENDPOINTS")
            .unwrap_or_default()
            .split(',')
            .map(|entry| entry.trim().to_string())
            .filter(|entry| !entry.is_empty())
            .collect::<Vec<_>>();

        let initial_inventory = env::var("SOLVER_INITIAL_INVENTORY")
            .unwrap_or_default()
            .split(',')
            .filter(|entry| !entry.trim().is_empty())
            .map(|entry| {
                let mut parts = entry.split(':');
                let asset = parts.next().unwrap_or("").trim().to_string();
                let amount = parts.next().unwrap_or("0").trim().parse::<u128>()
                    .map_err(|e| format!("Invalid inventory amount: {}", e))?;
                if asset.is_empty() {
                    return Err("Inventory asset must not be empty".to_string());
                }
                Ok((asset, amount))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let accept_all_intents = env::var("SOLVER_ACCEPT_ALL_INTENTS")
            .map(|v| v.trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let proof_service_url = env::var("PROOF_SERVICE_URL").ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let proof_service_api_key = env::var("PROOF_SERVICE_API_KEY").ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        Ok(Self {
            solver_address,
            gateway_url,
            gateway_api_key_header,
            gateway_api_key,
            solver_private_key,
            solver_account_address,
            solver_account_private_key,
            chain_id,
            batch_auction_address,
            batch_settlement_address,
            solver_bond_address,
            rpc_url,
            solver_fee_bps,
            solver_bond_proof,
            dex_quote_endpoints,
            initial_inventory,
            proof_service_url,
            proof_service_api_key,
            accept_all_intents,
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.gateway_url.starts_with("http://") && !self.gateway_url.starts_with("https://") {
            return Err("GATEWAY_URL must start with http:// or https://".to_string());
        }

        if self.gateway_api_key_header.trim().is_empty() {
            return Err("GATEWAY_API_KEY_HEADER must not be empty".to_string());
        }

        if self.gateway_api_key.trim().is_empty() {
            return Err("GATEWAY_API_KEY must not be empty".to_string());
        }

        if !self.solver_address.starts_with("0x") {
            return Err("SOLVER_ADDRESS must start with 0x".to_string());
        }

        if !self.solver_account_address.starts_with("0x") {
            return Err("SOLVER_ACCOUNT_ADDRESS must start with 0x".to_string());
        }

        if !self.solver_account_private_key.starts_with("0x") {
            return Err("SOLVER_ACCOUNT_PRIVATE_KEY must start with 0x".to_string());
        }
        if self.solver_account_private_key.len() != 66 {
            return Err("SOLVER_ACCOUNT_PRIVATE_KEY must be 32-byte hex with 0x prefix".to_string());
        }

        if self.chain_id.trim().is_empty() {
            return Err("STARKNET_CHAIN_ID must not be empty".to_string());
        }

        if !self.batch_auction_address.starts_with("0x") {
            return Err("BATCH_AUCTION_ADDRESS must start with 0x".to_string());
        }
        if !self.batch_settlement_address.starts_with("0x") {
            return Err("BATCH_SETTLEMENT_ADDRESS must start with 0x".to_string());
        }
        if !self.solver_bond_address.starts_with("0x") {
            return Err("SOLVER_BOND_ADDRESS must start with 0x".to_string());
        }

        if !self.rpc_url.starts_with("http://") && !self.rpc_url.starts_with("https://") {
            return Err("STARKNET_RPC_URL must start with http:// or https://".to_string());
        }

        if self.solver_fee_bps == 0 || self.solver_fee_bps > 10_000 {
            return Err("SOLVER_FEE_BPS must be between 1 and 10000".to_string());
        }

        if self.solver_bond_proof.trim().is_empty() {
            return Err("SOLVER_BOND_PROOF must not be empty".to_string());
        }

        if self.solver_private_key.trim().is_empty() {
            return Err("SOLVER_PRIVATE_KEY must not be empty".to_string());
        }
        if !self.solver_private_key.starts_with("0x") || self.solver_private_key.len() != 66 {
            return Err("SOLVER_PRIVATE_KEY must be 32-byte hex with 0x prefix".to_string());
        }

        if self.dex_quote_endpoints.is_empty() && self.initial_inventory.is_empty() && !self.accept_all_intents {
            return Err(
                "Solver has no fill source: set DEX_QUOTE_ENDPOINTS, SOLVER_INITIAL_INVENTORY, \
                 or SOLVER_ACCEPT_ALL_INTENTS=true".to_string()
            );
        }

        if let Some(url) = &self.proof_service_url {
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err("PROOF_SERVICE_URL must start with http:// or https://".to_string());
            }
        }

        Ok(())
    }
}
