use anyhow::Result;
use crate::starknet_client::{StarknetClient, StarknetConfig, SolverInfo};

pub struct BondManager {
    solver_address: String,
    client: StarknetClient,
}

impl BondManager {
    pub fn new(solver_address: String, config: StarknetConfig) -> Result<Self> {
        Ok(Self {
            solver_address,
            client: StarknetClient::new(config)?,
        })
    }

    pub async fn get_solver_info(&self) -> Result<SolverInfo> {
        self.client.get_solver_info(&self.solver_address).await
    }

    #[allow(dead_code)]
    pub async fn deposit_bond(&self, amount: u128) -> Result<String> {
        self.client.deposit_bond(amount).await
    }

    #[allow(dead_code)]
    pub async fn request_withdrawal(&self, amount: u128) -> Result<String> {
        self.client.request_withdrawal(amount).await
    }

    #[allow(dead_code)]
    pub async fn withdraw(&self, amount: u128) -> Result<String> {
        self.client.withdraw(amount).await
    }
}
