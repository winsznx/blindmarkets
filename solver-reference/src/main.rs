use anyhow::Result;
use tracing::info;
use tracing_subscriber;

mod intent_monitor;
mod matching_engine;
mod liquidity_aggregator;
mod solution_builder;
mod config;
mod starknet_client;
mod proof_provider;
mod bond_manager;

use intent_monitor::IntentMonitor;
use matching_engine::MatchingEngine;
use liquidity_aggregator::LiquidityAggregator;
use solution_builder::SolutionBuilder;
use config::SolverConfig;
use tokio::sync::mpsc;
use std::collections::HashMap;
use proof_provider::ProofProvider;
use bond_manager::BondManager;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    info!("🤖 Blind BTC Intent Markets - Reference Solver v1.0");
    info!("Initializing solver...");

    let config = SolverConfig::from_env()
        .map_err(|e| anyhow::anyhow!("Failed to load solver configuration: {}", e))?;
    config.validate()
        .map_err(|e| anyhow::anyhow!("Invalid solver configuration: {}", e))?;

    info!("Solver address: {}", config.solver_address);
    info!("Gateway URL: {}", config.gateway_url);

    let (intent_tx, mut intent_rx) = mpsc::channel(1000);
    let (batch_tx, mut batch_rx) = mpsc::channel(100);

    // Clone senders so channels stay open even when the WebSocket drops and reconnects.
    let intent_monitor = IntentMonitor::new(config.clone(), intent_tx.clone(), batch_tx.clone());
    tokio::spawn(async move {
        loop {
            match intent_monitor.connect_websocket().await {
                Ok(_) => tracing::warn!("WebSocket disconnected, reconnecting in 5s..."),
                Err(e) => tracing::error!("WebSocket error: {}, reconnecting in 5s...", e),
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    });

    let mut matching_engine = if config.accept_all_intents {
        tracing::warn!("SOLVER_ACCEPT_ALL_INTENTS=true: filling every intent at min_output (demo mode)");
        MatchingEngine::new().with_accept_all()
    } else {
        MatchingEngine::new()
    };
    for (asset, amount) in &config.initial_inventory {
        matching_engine.add_inventory(asset.clone(), *amount);
    }

    let liquidity_aggregator = LiquidityAggregator::new(config.dex_quote_endpoints.clone());
    let solution_builder = SolutionBuilder::new(
        config.solver_address.clone(),
        config.solver_fee_bps,
        config.solver_bond_proof.clone(),
        crate::starknet_client::StarknetConfig {
            rpc_url: config.rpc_url.clone(),
            chain_id: config.chain_id.clone(),
            account_address: config.solver_account_address.clone(),
            private_key: config.solver_account_private_key.clone(),
            batch_auction_address: config.batch_auction_address.clone(),
            batch_settlement_address: config.batch_settlement_address.clone(),
            solver_bond_address: config.solver_bond_address.clone(),
        },
    )?;

    let bond_manager = BondManager::new(
        config.solver_address.clone(),
        crate::starknet_client::StarknetConfig {
            rpc_url: config.rpc_url.clone(),
            chain_id: config.chain_id.clone(),
            account_address: config.solver_account_address.clone(),
            private_key: config.solver_account_private_key.clone(),
            batch_auction_address: config.batch_auction_address.clone(),
            batch_settlement_address: config.batch_settlement_address.clone(),
            solver_bond_address: config.solver_bond_address.clone(),
        },
    )?;

    if let Ok(info) = bond_manager.get_solver_info().await {
        if info.blacklisted {
            tracing::warn!("Solver is blacklisted on-chain");
        }
        if info.locked {
            tracing::warn!("Solver bond is locked on-chain");
        }
        tracing::info!(
            "Solver bond status: amount={}, success={}, failed={}",
            info.bond_amount,
            info.successful_settlements,
            info.failed_settlements
        );
    } else {
        tracing::warn!("Failed to fetch solver bond status");
    }

    let proof_provider = config.proof_service_url.clone()
        .map(|url| ProofProvider::new(url, config.proof_service_api_key.clone()));

    // Keyed by intent_id — survives batch reassignment between new_intent and batch_closed.
    let mut pending_intents: HashMap<String, crate::intent_monitor::DecryptedIntent> = HashMap::new();

    loop {
        tokio::select! {
            Some(envelope) = intent_rx.recv() => {
                pending_intents.insert(envelope.intent.intent_id.clone(), envelope.intent);
            }
            Some((batch_id, intent_count)) = batch_rx.recv() => {
                if intent_count == 0 {
                    info!("Batch {} closed with no intents", batch_id);
                    continue;
                }

                let ordered_intent_ids = match fetch_batch_intent_order(&config, &batch_id).await {
                    Ok(ids) => ids,
                    Err(e) => {
                        tracing::error!("Failed to fetch intent order for batch {}: {}", batch_id, e);
                        continue;
                    }
                };

                // Drain any buffered WS intents into pending_intents first.
                while let Ok(envelope) = intent_rx.try_recv() {
                    pending_intents.insert(envelope.intent.intent_id.clone(), envelope.intent);
                }

                // Check which intents are missing from the WS cache.
                let missing: Vec<_> = ordered_intent_ids.iter()
                    .filter(|id| !pending_intents.contains_key(id.as_str()))
                    .cloned()
                    .collect();

                // If any are missing, pull them directly from the gateway.
                if !missing.is_empty() {
                    tracing::warn!(
                        "Batch {} missing {} intent(s) from WS cache; fetching from gateway",
                        batch_id, missing.len()
                    );
                    match fetch_solver_intents(&config, &batch_id, &missing).await {
                        Ok(fetched) => {
                            for intent in fetched {
                                pending_intents.insert(intent.intent_id.clone(), intent);
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to fetch solver intents for batch {}: {}", batch_id, e);
                        }
                    }
                }

                let mut ordered_intents = Vec::new();
                for intent_id in &ordered_intent_ids {
                    if let Some(intent) = pending_intents.remove(intent_id.as_str()) {
                        ordered_intents.push(intent);
                    } else {
                        tracing::warn!("Missing intent {} for batch {} even after gateway fetch", intent_id, batch_id);
                    }
                }

                if ordered_intents.len() != ordered_intent_ids.len() {
                    tracing::warn!("Batch {} missing intents; skipping", batch_id);
                    continue;
                }

                if ordered_intents.is_empty() {
                    info!("Batch {} closed with no usable intents", batch_id);
                    continue;
                }

                info!("Processing {} intents in batch {}", ordered_intents.len(), batch_id);
                let mut fills = Vec::new();

                for intent in &ordered_intents {
                    if let Some(fill) = matching_engine.create_internal_fill(intent) {
                        info!("Filled intent {} internally", intent.intent_id);
                        fills.push(fill);
                    } else if let Some(fill) = liquidity_aggregator.find_best_route(intent).await {
                        info!("Filled intent {} via {}", intent.intent_id, fill.liquidity_source);
                        fills.push(fill);
                    } else {
                        info!("Could not fill intent {}", intent.intent_id);
                    }
                }

                if fills.len() != ordered_intents.len() {
                    info!("Not all intents could be filled for batch {}; skipping settlement", batch_id);
                    continue;
                }

                let execution_plan = match solution_builder.build_execution_plan(&ordered_intents, &fills) {
                    Ok(plan) => plan,
                    Err(e) => {
                        tracing::error!("Failed to build execution plan: {}", e);
                        continue;
                    }
                };

                let proofs = match build_proofs(&ordered_intents, &fills, proof_provider.as_ref(), config.accept_all_intents).await {
                    Ok(value) => value,
                    Err(e) => {
                        tracing::error!("Failed to build proofs for batch {}: {}", batch_id, e);
                        continue;
                    }
                };

                let solution = match solution_builder.build_solution(
                    batch_id.clone(),
                    fills,
                    execution_plan,
                    proofs,
                ) {
                    Ok(value) => value,
                    Err(e) => {
                        tracing::error!("Failed to build solution: {}", e);
                        continue;
                    }
                };

                match solution_builder.submit_solution(&solution).await {
                    Ok(tx_hash) => {
                        info!("✅ Solution submitted for batch {}", batch_id);
                        info!("   Transaction: {}", tx_hash);
                        info!("   Fills: {}", solution.fills.len());
                        info!("   Estimated surplus: {}", solution.estimated_surplus);
                        // Note: settlement is triggered by the coordinator after finalize_auction.
                        // Submitting it immediately here may revert; log but don't abort.
                        match solution_builder.submit_settlement(&solution).await {
                            Ok(settle_tx) => {
                                info!("✅ Settlement submitted for batch {}", batch_id);
                                info!("   Settlement tx: {}", settle_tx);
                            }
                            Err(e) => {
                                tracing::warn!("Settlement submission failed (may be expected before finalize): {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("❌ Solution submission failed for batch {}: {}", batch_id, e);
                    }
                }
            }
            else => {
                tracing::warn!("All solver channels closed; restarting in 5s...");
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                break;
            }
        }
    }
    Ok(())
}

async fn fetch_batch_intent_order(config: &SolverConfig, batch_id: &str) -> Result<Vec<String>> {
    let mut offset = 0;
    let mut all_ids = Vec::new();
    let client = reqwest::Client::new();

    loop {
        let url = format!(
            "{}/v1/batches/{}/intents?offset={}",
            config.gateway_url.trim_end_matches('/'),
            batch_id,
            offset
        );
        let response = client
            .get(url)
            .header(config.gateway_api_key_header.as_str(), config.gateway_api_key.as_str())
            .send()
            .await?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Gateway error: {}", body));
        }

        let payload: serde_json::Value = response.json().await?;
        let intents = payload.get("intent_ids")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("Invalid intent_ids response"))?;

        if intents.is_empty() {
            break;
        }

        for entry in intents {
            if let Some(id) = entry.as_str() {
                all_ids.push(id.to_string());
            }
        }

        offset += intents.len();
    }

    Ok(all_ids)
}

async fn fetch_solver_intents(
    config: &SolverConfig,
    batch_id: &str,
    intent_ids: &[String],
) -> Result<Vec<crate::intent_monitor::DecryptedIntent>> {
    use crate::intent_monitor::IntentMonitor;

    let monitor = IntentMonitor::new(
        config.clone(),
        tokio::sync::mpsc::channel(1).0,
        tokio::sync::mpsc::channel(1).0,
    );
    let solver_public_key = monitor.solver_public_key_hex()?;

    let base_url = format!(
        "{}/v1/batches/{}/solver_intents",
        config.gateway_url.trim_end_matches('/'),
        batch_id,
    );

    // Pass explicit intent_ids so the gateway can bypass the batch_id filter.
    // Intents may have been requeued to a different batch_id since batch_closed.
    let url = if !intent_ids.is_empty() {
        format!("{}?intent_ids={}", base_url, intent_ids.join(","))
    } else {
        base_url
    };

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header(config.gateway_api_key_header.as_str(), config.gateway_api_key.as_str())
        .header("x-solver-public-key", &solver_public_key)
        .send()
        .await?;

    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Gateway error: {}", body));
    }

    let payload: serde_json::Value = response.json().await?;
    let items = payload.get("intents")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("Invalid intents response"))?;

    let mut result = Vec::new();
    for item in items {
        let intent_id = item.get("intent_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let encrypted_data = item.get("encrypted_data").and_then(|v| v.as_str()).unwrap_or("");
        let gateway_public_key = item.get("gateway_public_key").and_then(|v| v.as_str()).unwrap_or("");

        match monitor.decrypt_intent_pub(encrypted_data, gateway_public_key) {
            Ok(mut decrypted) => {
                decrypted.intent_id = intent_id;
                result.push(decrypted);
            }
            Err(e) => {
                tracing::error!("Failed to decrypt fetched intent {}: {}", intent_id, e);
            }
        }
    }

    Ok(result)
}

async fn build_proofs(
    intents: &[crate::intent_monitor::DecryptedIntent],
    fills: &[crate::intent_monitor::Fill],
    provider: Option<&ProofProvider>,
    accept_all: bool,
) -> Result<Vec<String>> {
    let mut fill_map = std::collections::HashMap::new();
    for fill in fills {
        fill_map.insert(fill.intent_id.clone(), fill);
    }
    let mut proofs = Vec::with_capacity(intents.len());
    for intent in intents {
        if intent.privacy_mode == 0 {
            proofs.push("0x0".to_string());
            continue;
        }

        if provider.is_none() && accept_all {
            tracing::warn!(
                "Intent {} has privacy_mode={} but no proof service configured; using dummy proof (accept_all mode)",
                intent.intent_id, intent.privacy_mode
            );
            proofs.push("0x0".to_string());
            continue;
        }

        let proof_provider = provider
            .ok_or_else(|| anyhow::anyhow!("Proof service not configured for private intents"))?;
        let fill = fill_map
            .get(&intent.intent_id)
            .ok_or_else(|| anyhow::anyhow!("Missing fill for intent {}", intent.intent_id))?;
        let output_amount = compute_output_amount(fill, intent.min_output)?;
        let proof = proof_provider.fetch_proof(&intent.intent_id, output_amount).await?;
        proofs.push(proof);
    }
    Ok(proofs)
}

fn compute_output_amount(
    fill: &crate::intent_monitor::Fill,
    min_output: u128,
) -> Result<u128> {
    let numerator = fill.fill_amount
        .checked_mul(fill.execution_price)
        .ok_or_else(|| anyhow::anyhow!("Output amount overflow"))?;
    let output = numerator / 1_000_000;
    Ok(std::cmp::max(output, min_output))
}
