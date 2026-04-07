use serde::{Deserialize, Serialize};
use starknet::core::utils::get_selector_from_name;

#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    method: &'static str,
    params: JsonRpcCallParams<'a>,
    id: u64,
}

#[derive(Serialize)]
struct JsonRpcCallParams<'a> {
    request: CallRequest<'a>,
    block_id: &'static str,
}

#[derive(Serialize)]
struct CallRequest<'a> {
    contract_address: &'a str,
    entry_point_selector: &'a str,
    calldata: Vec<String>,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    result: Option<Vec<String>>,
    error: Option<serde_json::Value>,
}

/// Validate format-only (no network call). Used by tests and as pre-check before RPC.
pub fn validate_signature_format(
    user_address: &str,
    message_hash: &str,
    signature: &[String],
) -> Result<(), String> {
    if signature.len() < 2 {
        return Err("Invalid signature length".to_string());
    }
    if !user_address.starts_with("0x") || user_address.len() < 3 {
        return Err("Invalid user address: must start with 0x".to_string());
    }
    if !message_hash.starts_with("0x") || message_hash.len() < 3 {
        return Err("Invalid message hash: must start with 0x".to_string());
    }
    for (i, elem) in signature.iter().enumerate() {
        if !elem.starts_with("0x") || elem.len() < 3 {
            return Err(format!("Invalid signature element {}: must start with 0x", i));
        }
    }
    Ok(())
}

/// Verify a Starknet account signature via JSON-RPC `starknet_call` on `is_valid_signature`.
/// Works with both Cairo 1 and Cairo 2 account contracts (Argent X, Braavos, OpenZeppelin).
pub async fn verify_signature(
    rpc_url: &str,
    user_address: &str,
    message_hash: &str,
    signature: &[String],
) -> Result<bool, String> {
    validate_signature_format(user_address, message_hash, signature)?;

    let selector = get_selector_from_name("is_valid_signature")
        .map_err(|e| format!("Selector error: {}", e))?;

    let mut calldata = vec![
        message_hash.to_string(),
        format!("{:#x}", signature.len()),
    ];
    for elem in signature {
        calldata.push(elem.clone());
    }

    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        method: "starknet_call",
        params: JsonRpcCallParams {
            request: CallRequest {
                contract_address: user_address,
                entry_point_selector: &format!("{selector:#x}"),
                calldata,
            },
            block_id: "latest",
        },
        id: 1,
    };

    let response = reqwest::Client::new()
        .post(rpc_url)
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("RPC request failed: {}", e))?;

    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("RPC error: {}", body));
    }

    let rpc_resp: JsonRpcResponse = response
        .json()
        .await
        .map_err(|e| format!("RPC parse error: {}", e))?;

    if let Some(err) = rpc_resp.error {
        return Err(format!("RPC call error: {}", err));
    }

    // Cairo 2 accounts return 'VALID' (0x56414c4944); Cairo 1 returns non-zero for valid.
    let is_valid = rpc_resp
        .result
        .as_ref()
        .and_then(|r| r.first())
        .map(|v| v != "0x0")
        .unwrap_or(false);

    Ok(is_valid)
}

pub fn extract_user_from_signature(_signature: &[String]) -> Result<String, String> {
    Err("Cannot extract user from signature alone. User address must be provided separately.".to_string())
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod auth_tests;
