use axum::{
    extract::{ConnectInfo, Path, Query, Json},
    http::HeaderMap,
    http::StatusCode,
    response::IntoResponse,
    Extension,
};
use serde::{Deserialize, Serialize, Deserializer};
use std::net::SocketAddr;
use std::sync::Arc;
use std::str::FromStr;
use std::time::Instant;
use starknet::core::types::Felt;
use starknet_crypto::pedersen_hash;
use sqlx::types::BigDecimal;
use sqlx::{QueryBuilder, Postgres, FromRow};

use crate::config::Config;
use crate::rate_limiter::RateLimiter;
use crate::crypto;
use crate::starknet_client::{IntentCommitment, StarknetClient};
use crate::metrics;

#[derive(Debug, Deserialize)]
pub struct SubmitIntentRequest {
    pub intent_id: String,
    pub user_address: String,
    pub ciphertext: String,
    pub encrypted_session_key: String,
    pub commitment: String,
    pub user_signature: Vec<String>,
    pub client_public_key: String,
    pub nonce: String,
    pub authorization_hash: Option<String>,
    pub submission_mode: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SubmitIntentResponse {
    pub intent_id: String,
    pub batch_id: String,
    pub estimated_execution_time: u64,
    pub awaiting_user_transaction: bool,
}

#[derive(Debug, Serialize)]
pub struct GatewayPublicKeyResponse {
    pub gateway_public_key: String,
}

#[derive(Debug, Deserialize)]
struct DecryptedIntent {
    pub user_address: String,
    pub asset_in: String,
    pub asset_out: String,
    #[serde(deserialize_with = "deserialize_u128")]
    pub amount: u128,
    pub amount_commitment: String,
    #[serde(deserialize_with = "deserialize_u128")]
    pub min_output: u128,
    pub max_fee_bps: u16,
    #[serde(deserialize_with = "deserialize_u64")]
    pub deadline: u64,
    pub privacy_mode: u8,
    pub nonce: String,
}

#[derive(Debug, Serialize)]
pub struct IntentStatusResponse {
    pub intent_id: String,
    pub status: String,
    pub batch_id: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct IntentListItem {
    pub intent_id: String,
    pub user_address: String,
    pub status: String,
    pub batch_id: String,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Debug, Serialize)]
pub struct IntentListResponse {
    pub intents: Vec<IntentListItem>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Serialize)]
pub struct BatchIntentListResponse {
    pub intent_ids: Vec<String>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct BatchListItem {
    pub batch_id: String,
    pub close_time: i64,
    pub intent_count: i32,
    pub auction_deadline: i64,
    pub status: String,
    pub created_at: chrono::NaiveDateTime,
    pub settled_at: Option<chrono::NaiveDateTime>,
    pub failure_reason: Option<String>,
    pub failed_at: Option<chrono::NaiveDateTime>,
}

#[derive(Debug, Serialize)]
pub struct BatchListResponse {
    pub batches: Vec<BatchListItem>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubmissionMode {
    Gateway,
    SelfCommit,
}

impl SubmissionMode {
    fn as_db_value(self) -> &'static str {
        match self {
            SubmissionMode::Gateway => "GATEWAY",
            SubmissionMode::SelfCommit => "SELF_COMMIT",
        }
    }
}

fn initial_intent_status(mode: SubmissionMode) -> &'static str {
    match mode {
        SubmissionMode::Gateway => "PENDING",
        SubmissionMode::SelfCommit => "AWAITING_ONCHAIN",
    }
}

pub async fn submit_intent(
    axum::extract::State(pool): axum::extract::State<sqlx::PgPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(rate_limiter): Extension<Arc<RateLimiter>>,
    Extension(broadcaster): Extension<Arc<crate::websocket::SolverBroadcaster>>,
    Extension(starknet_client): Extension<Arc<StarknetClient>>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(payload): Json<SubmitIntentRequest>,
) -> Result<Json<SubmitIntentResponse>, StatusCode> {
    let metrics = metrics::init_metrics();
    let start = Instant::now();
    let reject = |code: StatusCode| {
        metrics.intents_rejected_total.inc();
        metrics.request_latency_seconds.observe(start.elapsed().as_secs_f64());
        Err(code)
    };
    require_api_key(&headers, &config)?;

    if let Err(code) = validate_submit_request(&payload) {
        return reject(code);
    }
    let submission_mode = parse_submission_mode(payload.submission_mode.as_deref())?;

    let client_ip = client_addr.ip().to_string();
    rate_limiter.check_ip_limit(&client_ip).await.map_err(|e| {
        tracing::warn!("IP rate limit exceeded: {}", e);
        StatusCode::TOO_MANY_REQUESTS
    })?;

    rate_limiter.check_user_limit(&payload.user_address).await.map_err(|e| {
        tracing::warn!("User rate limit exceeded: {}", e);
        StatusCode::TOO_MANY_REQUESTS
    })?;

    tracing::info!("Received intent submission: {}", payload.intent_id);

    let message_hash = match submission_mode {
        SubmissionMode::Gateway => payload.commitment.as_str(),
        SubmissionMode::SelfCommit => payload.authorization_hash.as_deref().ok_or_else(|| {
            tracing::warn!("authorization_hash is required for self_commit mode");
            StatusCode::BAD_REQUEST
        })?,
    };
    let user_address_from_sig = extract_user_address_from_intent(&payload)?;

    let is_valid = crate::auth::verify_signature(
        &config.starknet.rpc_url,
        &user_address_from_sig,
        message_hash,
        &payload.user_signature
    ).await.map_err(|e| {
        tracing::error!("Signature verification failed: {}", e);
        StatusCode::UNAUTHORIZED
    })?;

    if !is_valid {
        tracing::warn!("Invalid signature for intent {}", payload.intent_id);
        return reject(StatusCode::UNAUTHORIZED);
    }

    let batch_id = compute_current_batch_id(
        config.intents.genesis_timestamp,
        config.intents.batch_window_seconds
    );
    let estimated_execution_time = compute_execution_time(
        config.intents.genesis_timestamp,
        config.intents.batch_window_seconds,
        config.intents.execution_delay_seconds
    );

    let decrypted = crypto::decrypt_from_client(
        &payload.ciphertext,
        &payload.encrypted_session_key,
        &payload.client_public_key,
        &config.security.gateway_private_key,
    ).map_err(|e| {
        tracing::error!("Failed to decrypt intent: {}", e);
        StatusCode::BAD_REQUEST
    })?;

    let decrypted_intent: DecryptedIntent = serde_json::from_slice(&decrypted).map_err(|e| {
        tracing::error!("Failed to parse decrypted intent: {}", e);
        StatusCode::BAD_REQUEST
    })?;

    if decrypted_intent.user_address != payload.user_address {
        tracing::warn!("Decrypted user address mismatch for intent {}", payload.intent_id);
        return reject(StatusCode::BAD_REQUEST);
    }

    if !is_hex_felt(&decrypted_intent.asset_in) {
        tracing::warn!("Invalid asset_in format for intent {}", payload.intent_id);
        return reject(StatusCode::BAD_REQUEST);
    }

    if decrypted_intent.amount == 0 {
        tracing::warn!("Invalid amount for intent {}", payload.intent_id);
        return reject(StatusCode::BAD_REQUEST);
    }

    if !is_hex_felt(&decrypted_intent.amount_commitment) {
        tracing::warn!("Invalid amount_commitment for intent {}", payload.intent_id);
        return reject(StatusCode::BAD_REQUEST);
    }

    if !is_hex_felt(&decrypted_intent.nonce) {
        tracing::warn!("Invalid nonce format for intent {}", payload.intent_id);
        return reject(StatusCode::BAD_REQUEST);
    }

    if decrypted_intent.nonce != payload.nonce {
        tracing::warn!("Nonce mismatch for intent {}", payload.intent_id);
        return reject(StatusCode::BAD_REQUEST);
    }

    if decrypted_intent.asset_out.is_empty() || !is_hex_felt(&decrypted_intent.asset_out) {
        tracing::warn!("Invalid asset_out format for intent {}", payload.intent_id);
        return reject(StatusCode::BAD_REQUEST);
    }

    if decrypted_intent.min_output == 0 {
        tracing::warn!("Invalid min_output for intent {}", payload.intent_id);
        return reject(StatusCode::BAD_REQUEST);
    }

    if decrypted_intent.max_fee_bps > 10_000 {
        tracing::warn!("Invalid max_fee_bps for intent {}", payload.intent_id);
        return reject(StatusCode::BAD_REQUEST);
    }

    if decrypted_intent.privacy_mode > 2 {
        tracing::warn!("Invalid privacy_mode for intent {}", payload.intent_id);
        return reject(StatusCode::BAD_REQUEST);
    }

    let computed_amount_commitment = compute_amount_commitment(
        decrypted_intent.amount,
        &decrypted_intent.nonce,
    )?;
    let provided_amount_commitment =
        field_element_from_hex(&decrypted_intent.amount_commitment, "amount_commitment")?;
    if computed_amount_commitment != provided_amount_commitment {
        tracing::warn!("Amount commitment mismatch for intent {}", payload.intent_id);
        return reject(StatusCode::BAD_REQUEST);
    }

    let computed_intent_hash = compute_intent_hash(
        &payload.user_address,
        &decrypted_intent.asset_in,
        &decrypted_intent.asset_out,
        computed_amount_commitment,
        decrypted_intent.min_output,
        decrypted_intent.max_fee_bps,
        decrypted_intent.deadline,
        decrypted_intent.privacy_mode,
        &decrypted_intent.nonce,
    )?;
    let provided_intent_hash = field_element_from_hex(&payload.commitment, "intent_hash")?;
    if computed_intent_hash != provided_intent_hash {
        tracing::warn!("Intent hash mismatch for intent {}", payload.intent_id);
        return reject(StatusCode::BAD_REQUEST);
    }

    let computed_intent_id = compute_intent_id(
        &payload.user_address,
        &decrypted_intent.nonce,
        computed_intent_hash,
    )?;
    let provided_intent_id = field_element_from_hex(&payload.intent_id, "intent_id")?;
    if computed_intent_id != provided_intent_id {
        tracing::warn!("Intent ID mismatch for intent {}", payload.intent_id);
        return reject(StatusCode::BAD_REQUEST);
    }

    let pending_amount = BigDecimal::from_str(&decrypted_intent.amount.to_string()).map_err(|e| {
        tracing::error!("Invalid amount format: {}", e);
        StatusCode::BAD_REQUEST
    })?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .as_secs();
    if decrypted_intent.deadline <= now {
        tracing::warn!("Intent deadline already passed for {}", payload.intent_id);
        return reject(StatusCode::BAD_REQUEST);
    }
    if decrypted_intent.deadline > now.saturating_add(config.intents.intent_deadline_seconds) {
        tracing::warn!("Intent deadline too far in future for {}", payload.intent_id);
        return reject(StatusCode::BAD_REQUEST);
    }

    let mut tx = pool.begin().await.map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let nonce_insert = sqlx::query!(
        r#"
        INSERT INTO user_nonces (user_address, nonce)
        VALUES ($1, $2)
        ON CONFLICT DO NOTHING
        "#,
        payload.user_address,
        decrypted_intent.nonce
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if nonce_insert.rows_affected() == 0 {
        tracing::warn!("Replay nonce detected for user {}", payload.user_address);
        return reject(StatusCode::CONFLICT);
    }

    sqlx::query!(
        r#"
        INSERT INTO pending_balances (user_address, asset_address, pending_amount)
        VALUES ($1, $2, $3)
        ON CONFLICT (user_address, asset_address)
        DO UPDATE SET pending_amount = pending_balances.pending_amount + EXCLUDED.pending_amount
        "#,
        payload.user_address,
        decrypted_intent.asset_in,
        pending_amount,
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let intent_insert = sqlx::query!(
        r#"
        INSERT INTO intents (
            intent_id,
            user_address,
            asset_in,
            asset_out,
            amount,
            amount_commitment,
            min_output,
            max_fee_bps,
            privacy_mode,
            nonce,
            ciphertext,
            encrypted_session_key,
            commitment,
            client_public_key,
            batch_id,
            deadline,
            submission_mode,
            status
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
        ON CONFLICT (intent_id) DO NOTHING
        "#,
        payload.intent_id,
        payload.user_address,
        decrypted_intent.asset_in,
        decrypted_intent.asset_out,
        pending_amount,
        decrypted_intent.amount_commitment,
        BigDecimal::from_str(&decrypted_intent.min_output.to_string()).map_err(|e| {
            tracing::error!("Invalid min_output format: {}", e);
            StatusCode::BAD_REQUEST
        })?,
        decrypted_intent.max_fee_bps as i32,
        decrypted_intent.privacy_mode as i16,
        payload.nonce,
        payload.ciphertext,
        payload.encrypted_session_key,
        payload.commitment,
        payload.client_public_key,
        batch_id,
        decrypted_intent.deadline as i64,
        submission_mode.as_db_value(),
        initial_intent_status(submission_mode),
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if intent_insert.rows_affected() == 0 {
        tx.rollback().await.ok();
        tracing::warn!("Intent {} already exists", payload.intent_id);
        return reject(StatusCode::CONFLICT);
    }

    tx.commit().await.map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if submission_mode == SubmissionMode::Gateway {
        let commitment = IntentCommitment {
            intent_id: payload.intent_id.clone(),
            user_address: payload.user_address.clone(),
            intent_hash: payload.commitment.clone(),
            nonce: payload.nonce.clone(),
            asset_in: decrypted_intent.asset_in.clone(),
            asset_out: decrypted_intent.asset_out.clone(),
            amount_commitment: decrypted_intent.amount_commitment.clone(),
            min_output: decrypted_intent.min_output.to_string(),
            max_fee_bps: decrypted_intent.max_fee_bps,
            deadline: decrypted_intent.deadline,
            privacy_mode: decrypted_intent.privacy_mode,
        };

        match starknet_client
            .commit_intent(commitment, payload.user_signature.clone())
            .await
        {
            Ok(tx_hash) => {
                let committed_at = chrono::Utc::now().naive_utc();
                sqlx::query!(
                    r#"
                    UPDATE intents
                    SET onchain_tx_hash = $2,
                        onchain_committed_at = COALESCE(onchain_committed_at, $3)
                    WHERE intent_id = $1
                    "#,
                    payload.intent_id,
                    tx_hash,
                    committed_at,
                )
                .execute(&pool)
                .await
                .map_err(|e| {
                    tracing::error!("Database error: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;

                tracing::info!(
                    "Intent {} committed on-chain: {}",
                    payload.intent_id,
                    tx_hash
                );
            }
            Err(e) => {
                tracing::error!("On-chain commit failed for {}: {}", payload.intent_id, e);
                if let Ok(mut rollback_tx) = pool.begin().await {
                    let _ = sqlx::query!(
                        r#"
                        UPDATE intents
                        SET status = 'ONCHAIN_FAILED'
                        WHERE intent_id = $1
                        "#,
                        payload.intent_id
                    )
                    .execute(&mut *rollback_tx)
                    .await;

                    let _ = sqlx::query!(
                        r#"
                        DELETE FROM user_nonces
                        WHERE user_address = $1 AND nonce = $2
                        "#,
                        payload.user_address,
                        payload.nonce
                    )
                    .execute(&mut *rollback_tx)
                    .await;

                    let _ = sqlx::query!(
                        r#"
                        UPDATE pending_balances
                        SET pending_amount = GREATEST(pending_amount - $3, 0)
                        WHERE user_address = $1 AND asset_address = $2
                        "#,
                        payload.user_address,
                        decrypted_intent.asset_in,
                        pending_amount
                    )
                    .execute(&mut *rollback_tx)
                    .await;

                    let _ = rollback_tx.commit().await;
                }
                metrics.intents_onchain_failed_total.inc();
                metrics.request_latency_seconds.observe(start.elapsed().as_secs_f64());
                return Err(StatusCode::BAD_GATEWAY);
            }
        }

        tracing::info!("Intent {} saved to database", payload.intent_id);

        crate::websocket::broadcast_new_intent(
            &broadcaster,
            payload.intent_id.clone(),
            batch_id.clone(),
            decrypted,
        ).await;
    } else {
        tracing::info!("Intent {} stored awaiting user on-chain commit", payload.intent_id);
    }

    metrics.intents_submitted_total.inc();
    metrics.request_latency_seconds.observe(start.elapsed().as_secs_f64());

    Ok(Json(SubmitIntentResponse {
        intent_id: payload.intent_id,
        batch_id,
        estimated_execution_time,
        awaiting_user_transaction: submission_mode == SubmissionMode::SelfCommit,
    }))
}

pub async fn get_intent_status(
    Path(intent_id): Path<String>,
    axum::extract::State(pool): axum::extract::State<sqlx::PgPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(rate_limiter): Extension<Arc<RateLimiter>>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Json<IntentStatusResponse>, StatusCode> {
    require_api_key(&headers, &config)?;

    let client_ip = client_addr.ip().to_string();
    rate_limiter.check_ip_limit(&client_ip).await.map_err(|e| {
        tracing::warn!("IP rate limit exceeded: {}", e);
        StatusCode::TOO_MANY_REQUESTS
    })?;

    tracing::info!("Querying status for intent: {}", intent_id);
    
    let result = sqlx::query!(
        r#"
        SELECT status, batch_id
        FROM intents
        WHERE intent_id = $1
        "#,
        intent_id
    )
    .fetch_optional(&pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    match result {
        Some(row) => {
            Ok(Json(IntentStatusResponse {
                intent_id,
                status: row.status,
                batch_id: Some(row.batch_id),
            }))
        }
        None => {
            tracing::warn!("Intent {} not found", intent_id);
            Err(StatusCode::NOT_FOUND)
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ListIntentsQuery {
    pub user_address: Option<String>,
    pub status: Option<String>,
    pub batch_id: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

pub async fn list_intents(
    axum::extract::State(pool): axum::extract::State<sqlx::PgPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(rate_limiter): Extension<Arc<RateLimiter>>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(params): Query<ListIntentsQuery>,
) -> Result<Json<IntentListResponse>, StatusCode> {
    require_api_key(&headers, &config)?;

    let client_ip = client_addr.ip().to_string();
    rate_limiter.check_ip_limit(&client_ip).await.map_err(|e| {
        tracing::warn!("IP rate limit exceeded: {}", e);
        StatusCode::TOO_MANY_REQUESTS
    })?;

    if let Some(ref user_address) = params.user_address {
        if !is_hex_felt(user_address) {
            tracing::warn!("Invalid user_address format");
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    if let Some(ref status) = params.status {
        let normalized = status.to_uppercase();
        if !matches!(
            normalized.as_str(),
            "PENDING" | "AUCTION" | "SETTLED" | "CANCELED" | "ONCHAIN_FAILED" | "AWAITING_ONCHAIN"
        ) {
            tracing::warn!("Invalid status filter");
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    if let Some(ref batch_id) = params.batch_id {
        if !batch_id.chars().all(|c| c.is_ascii_digit()) {
            tracing::warn!("Invalid batch_id filter");
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    let limit = params.limit.unwrap_or(config.server.max_page_size);
    if limit == 0 || limit > config.server.max_page_size {
        tracing::warn!("Invalid limit");
        return Err(StatusCode::BAD_REQUEST);
    }

    let offset = params.offset.unwrap_or(0);

    let mut query = QueryBuilder::<Postgres>::new(
        "SELECT intent_id, user_address, status, batch_id, created_at FROM intents"
    );
    let mut has_where = false;

    if let Some(user) = params.user_address {
        query.push(" WHERE user_address = ");
        query.push_bind(user);
        has_where = true;
    }

    if let Some(status) = params.status {
        query.push(if has_where { " AND " } else { " WHERE " });
        query.push("status = ");
        query.push_bind(status.to_uppercase());
        has_where = true;
    }

    if let Some(batch_id) = params.batch_id {
        query.push(if has_where { " AND " } else { " WHERE " });
        query.push("batch_id = ");
        query.push_bind(batch_id);
        has_where = true;
    }

    query.push(" ORDER BY created_at DESC LIMIT ");
    query.push_bind(limit as i64);
    query.push(" OFFSET ");
    query.push_bind(offset as i64);

    let rows = query
        .build_query_as::<IntentListItem>()
        .fetch_all(&pool)
        .await
        .map_err(|e| {
            tracing::error!("Database error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(IntentListResponse {
        intents: rows,
        limit,
        offset,
    }))
}

#[derive(Debug, Deserialize)]
pub struct BatchIntentsQuery {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

pub async fn list_batch_intents(
    Path(batch_id): Path<String>,
    axum::extract::State(pool): axum::extract::State<sqlx::PgPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(rate_limiter): Extension<Arc<RateLimiter>>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(params): Query<BatchIntentsQuery>,
) -> Result<Json<BatchIntentListResponse>, StatusCode> {
    require_api_key(&headers, &config)?;

    let client_ip = client_addr.ip().to_string();
    rate_limiter.check_ip_limit(&client_ip).await.map_err(|e| {
        tracing::warn!("IP rate limit exceeded: {}", e);
        StatusCode::TOO_MANY_REQUESTS
    })?;

    if !batch_id.chars().all(|c| c.is_ascii_digit()) {
        tracing::warn!("Invalid batch_id format");
        return Err(StatusCode::BAD_REQUEST);
    }

    let limit = params.limit.unwrap_or(config.server.max_page_size);
    if limit == 0 || limit > config.server.max_page_size {
        tracing::warn!("Invalid limit");
        return Err(StatusCode::BAD_REQUEST);
    }
    let offset = params.offset.unwrap_or(0);

    let rows = sqlx::query!(
        r#"
        SELECT intent_id
        FROM intents
        WHERE batch_id = $1
          AND status IN ('PENDING', 'AUCTION')
        ORDER BY sequence ASC NULLS LAST
        LIMIT $2 OFFSET $3
        "#,
        batch_id,
        limit as i64,
        offset as i64
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let intent_ids = rows.into_iter().map(|row| row.intent_id).collect::<Vec<_>>();

    Ok(Json(BatchIntentListResponse { intent_ids }))
}

#[derive(Debug, Serialize)]
pub struct SolverEncryptedIntent {
    pub intent_id: String,
    pub encrypted_data: String,
    pub gateway_public_key: String,
}

#[derive(Debug, Serialize)]
pub struct SolverBatchIntentsResponse {
    pub intents: Vec<SolverEncryptedIntent>,
}

/// Called by solvers to pull encrypted intents for a batch on-demand.
/// Re-encrypts each intent's plaintext for the requesting solver.
#[derive(Debug, Deserialize)]
pub struct SolverBatchIntentsQuery {
    /// Comma-separated intent_ids. When provided, bypasses the batch_id filter
    /// so the solver can look up intents that may have been requeued to a
    /// different batch since the batch_closed message was sent.
    pub intent_ids: Option<String>,
}

pub async fn get_solver_batch_intents(
    Path(batch_id): Path<String>,
    axum::extract::State(pool): axum::extract::State<sqlx::PgPool>,
    Extension(config): Extension<Arc<Config>>,
    headers: HeaderMap,
    Query(params): Query<SolverBatchIntentsQuery>,
) -> Result<Json<SolverBatchIntentsResponse>, StatusCode> {
    require_api_key(&headers, &config)?;

    let solver_public_key = headers
        .get("x-solver-public-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !solver_public_key.starts_with("0x") || solver_public_key.len() != 66 {
        tracing::warn!("Missing or invalid x-solver-public-key header");
        return Err(StatusCode::BAD_REQUEST);
    }

    if !batch_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(StatusCode::BAD_REQUEST);
    }

    // If explicit intent_ids are provided, look them up directly regardless of
    // current batch_id assignment — the intent may have been requeued between
    // batch_closed and this fetch.
    let explicit_ids: Option<Vec<String>> = params.intent_ids.as_deref().map(|raw| {
        raw.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });

    struct SolverIntentRow {
        intent_id: String,
        #[allow(dead_code)]
        batch_id: String,
        ciphertext: String,
        encrypted_session_key: Option<String>,
        client_public_key: String,
    }

    let rows: Vec<SolverIntentRow> = if let Some(ref ids) = explicit_ids {
        tracing::info!(
            "Fetching {} intent(s) by explicit IDs for batch {} (bypassing batch_id filter)",
            ids.len(), batch_id
        );
        sqlx::query_as!(
            SolverIntentRow,
            r#"
            SELECT intent_id, batch_id, ciphertext, encrypted_session_key, client_public_key
            FROM intents
            WHERE intent_id = ANY($1)
              AND status IN ('PENDING', 'AUCTION')
              AND encrypted_session_key IS NOT NULL
            ORDER BY sequence ASC NULLS LAST
            "#,
            ids as &[String]
        )
        .fetch_all(&pool)
        .await
        .map_err(|e| {
            tracing::error!("Database error (explicit ids): {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
    } else {
        sqlx::query_as!(
            SolverIntentRow,
            r#"
            SELECT intent_id, batch_id, ciphertext, encrypted_session_key, client_public_key
            FROM intents
            WHERE batch_id = $1
              AND status IN ('PENDING', 'AUCTION')
              AND encrypted_session_key IS NOT NULL
            ORDER BY sequence ASC NULLS LAST
            "#,
            batch_id
        )
        .fetch_all(&pool)
        .await
        .map_err(|e| {
            tracing::error!("Database error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
    };

    let mut intents = Vec::new();
    for row in rows {
        let Some(enc_session_key) = row.encrypted_session_key else {
            continue;
        };

        let plaintext = match crate::crypto::decrypt_from_client(
            &row.ciphertext,
            &enc_session_key,
            &row.client_public_key,
            &config.security.gateway_private_key,
        ) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to decrypt intent {} for solver fetch: {}", row.intent_id, e);
                continue;
            }
        };

        let payload = match crate::crypto::encrypt_for_solver(
            &plaintext,
            &config.security.gateway_private_key,
            solver_public_key,
        ) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to re-encrypt intent {} for solver: {}", row.intent_id, e);
                continue;
            }
        };

        intents.push(SolverEncryptedIntent {
            intent_id: row.intent_id,
            encrypted_data: payload.ciphertext_hex,
            gateway_public_key: payload.sender_public_key_hex,
        });
    }

    tracing::info!("Returning {} encrypted intents for batch {} to solver", intents.len(), batch_id);
    Ok(Json(SolverBatchIntentsResponse { intents }))
}

#[derive(Debug, Deserialize)]
pub struct ListBatchesQuery {
    pub status: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

pub async fn list_batches(
    axum::extract::State(pool): axum::extract::State<sqlx::PgPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(rate_limiter): Extension<Arc<RateLimiter>>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(params): Query<ListBatchesQuery>,
) -> Result<Json<BatchListResponse>, StatusCode> {
    require_api_key(&headers, &config)?;

    let client_ip = client_addr.ip().to_string();
    rate_limiter.check_ip_limit(&client_ip).await.map_err(|e| {
        tracing::warn!("IP rate limit exceeded: {}", e);
        StatusCode::TOO_MANY_REQUESTS
    })?;

    if let Some(ref status) = params.status {
        let normalized = status.to_uppercase();
        if !matches!(normalized.as_str(), "FORMING" | "AUCTION" | "SETTLED" | "FAILED") {
            tracing::warn!("Invalid batch status filter");
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    let limit = params.limit.unwrap_or(config.server.max_page_size);
    if limit == 0 || limit > config.server.max_page_size {
        tracing::warn!("Invalid limit");
        return Err(StatusCode::BAD_REQUEST);
    }

    let offset = params.offset.unwrap_or(0);

    let mut query = QueryBuilder::<Postgres>::new(
        "SELECT batch_id, close_time, intent_count, auction_deadline, status, created_at, settled_at, failure_reason, failed_at FROM batches"
    );

    if let Some(status) = params.status {
        query.push(" WHERE status = ");
        query.push_bind(status.to_uppercase());
    }

    query.push(" ORDER BY close_time DESC LIMIT ");
    query.push_bind(limit as i64);
    query.push(" OFFSET ");
    query.push_bind(offset as i64);

    let rows = query
        .build_query_as::<BatchListItem>()
        .fetch_all(&pool)
        .await
        .map_err(|e| {
            tracing::error!("Database error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(BatchListResponse {
        batches: rows,
        limit,
        offset,
    }))
}

#[derive(Debug, Deserialize)]
pub struct CancelIntentRequest {
    pub user_address: String,
    pub signature: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct OnchainLifecycleRequest {
    pub action: String,
    pub user_address: String,
    pub tx_hash: String,
}

pub async fn cancel_intent(
    Path(intent_id): Path<String>,
    axum::extract::State(pool): axum::extract::State<sqlx::PgPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(rate_limiter): Extension<Arc<RateLimiter>>,
    Extension(starknet_client): Extension<Arc<StarknetClient>>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(payload): Json<CancelIntentRequest>,
) -> Result<StatusCode, StatusCode> {
    require_api_key(&headers, &config)?;

    let client_ip = client_addr.ip().to_string();
    rate_limiter.check_ip_limit(&client_ip).await.map_err(|e| {
        tracing::warn!("IP rate limit exceeded: {}", e);
        StatusCode::TOO_MANY_REQUESTS
    })?;

    tracing::info!("Canceling intent: {} for user: {}", intent_id, payload.user_address);

    if !payload.user_address.starts_with("0x") || !is_hex_felt(&payload.user_address) {
        tracing::warn!("Invalid user_address for cancel request");
        return Err(StatusCode::BAD_REQUEST);
    }
    if payload.signature.len() != 2 {
        tracing::warn!("signature must include r and s for cancel request");
        return Err(StatusCode::BAD_REQUEST);
    }
    if !is_hex_felt(&payload.signature[0]) || !is_hex_felt(&payload.signature[1]) {
        tracing::warn!("signature values must be valid felt hex");
        return Err(StatusCode::BAD_REQUEST);
    }
    
    // Query intent from database
    let intent = sqlx::query!(
        r#"
        SELECT user_address, status, ciphertext, client_public_key, encrypted_session_key
        FROM intents
        WHERE intent_id = $1
        "#,
        intent_id
    )
    .fetch_optional(&pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let intent = match intent {
        Some(i) => i,
        None => {
            tracing::warn!("Intent {} not found", intent_id);
            return Err(StatusCode::NOT_FOUND);
        }
    };

    // Verify user owns this intent
    if intent.user_address != payload.user_address {
        tracing::warn!("User {} does not own intent {}", payload.user_address, intent_id);
        return Err(StatusCode::FORBIDDEN);
    }

    // Check intent is still cancellable
    if !matches!(intent.status.as_str(), "PENDING" | "AUCTION") {
        tracing::warn!("Intent {} cannot be cancelled (status: {})", intent_id, intent.status);
        return Err(StatusCode::BAD_REQUEST);
    }

    let encrypted_session_key = intent.encrypted_session_key.ok_or_else(|| {
        tracing::error!("Missing encrypted_session_key for intent {}", intent_id);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let decrypted = crypto::decrypt_from_client(
        &intent.ciphertext,
        &encrypted_session_key,
        &intent.client_public_key,
        &config.security.gateway_private_key,
    ).map_err(|e| {
        tracing::error!("Failed to decrypt intent for cancellation: {}", e);
        StatusCode::BAD_REQUEST
    })?;

    let decrypted_intent: DecryptedIntent = serde_json::from_slice(&decrypted).map_err(|e| {
        tracing::error!("Failed to parse decrypted intent for cancellation: {}", e);
        StatusCode::BAD_REQUEST
    })?;
    if decrypted_intent.user_address != payload.user_address {
        tracing::warn!("Decrypted user address mismatch for cancel {}", intent_id);
        return Err(StatusCode::BAD_REQUEST);
    }

    // Verify signature
    let message = intent_id.clone();
    let is_valid = crate::auth::verify_signature(
        &config.starknet.rpc_url,
        &payload.user_address,
        &message,
        &payload.signature
    ).await.map_err(|e| {
        tracing::error!("Signature verification failed: {}", e);
        StatusCode::UNAUTHORIZED
    })?;

    if !is_valid {
        tracing::warn!("Invalid signature for cancel request");
        return Err(StatusCode::UNAUTHORIZED);
    }

    let tx_hash = match starknet_client
        .cancel_intent(&intent_id, payload.signature.clone())
        .await
    {
        Ok(tx_hash) => {
            tracing::info!("Intent {} canceled on-chain: {}", intent_id, tx_hash);
            tx_hash
        }
        Err(e) => {
            tracing::error!("On-chain cancel failed for {}: {}", intent_id, e);
            return Err(StatusCode::BAD_GATEWAY);
        }
    };

    let pending_amount = BigDecimal::from_str(&decrypted_intent.amount.to_string()).map_err(|e| {
        tracing::error!("Invalid amount format: {}", e);
        StatusCode::BAD_REQUEST
    })?;
    let canceled_at = chrono::Utc::now().naive_utc();

    let mut tx = pool.begin().await.map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Update status in database
    sqlx::query!(
        r#"
        UPDATE intents
        SET status = 'CANCELED',
            onchain_tx_hash = $2,
            canceled_at = $3
        WHERE intent_id = $1
        "#,
        intent_id,
        tx_hash,
        canceled_at,
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    sqlx::query!(
        r#"
        UPDATE pending_balances
        SET pending_amount = GREATEST(pending_amount - $3, 0)
        WHERE user_address = $1 AND asset_address = $2
        "#,
        payload.user_address,
        decrypted_intent.asset_in,
        pending_amount,
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tx.commit().await.map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tracing::info!("Intent {} cancelled successfully", intent_id);
    Ok(StatusCode::OK)
}

pub async fn reconcile_onchain_intent(
    Path(intent_id): Path<String>,
    axum::extract::State(pool): axum::extract::State<sqlx::PgPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(rate_limiter): Extension<Arc<RateLimiter>>,
    Extension(broadcaster): Extension<Arc<crate::websocket::SolverBroadcaster>>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(payload): Json<OnchainLifecycleRequest>,
) -> Result<StatusCode, StatusCode> {
    require_api_key(&headers, &config)?;

    let client_ip = client_addr.ip().to_string();
    rate_limiter.check_ip_limit(&client_ip).await.map_err(|e| {
        tracing::warn!("IP rate limit exceeded: {}", e);
        StatusCode::TOO_MANY_REQUESTS
    })?;

    validate_onchain_lifecycle_request(&payload)?;

    let action = payload.action.trim().to_ascii_uppercase();
    let intent = sqlx::query!(
        r#"
        SELECT
            user_address,
            status,
            batch_id,
            ciphertext,
            encrypted_session_key,
            client_public_key,
            asset_in,
            amount,
            submission_mode,
            onchain_tx_hash
        FROM intents
        WHERE intent_id = $1
        "#,
        intent_id
    )
    .fetch_optional(&pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let intent = match intent {
        Some(intent) => intent,
        None => return Err(StatusCode::NOT_FOUND),
    };

    if intent.user_address != payload.user_address {
        tracing::warn!("User {} does not own intent {}", payload.user_address, intent_id);
        return Err(StatusCode::FORBIDDEN);
    }

    let now = chrono::Utc::now().naive_utc();
    match action.as_str() {
        "COMMITTED" => {
            if intent
                .onchain_tx_hash
                .as_deref()
                .is_some_and(|existing| existing.eq_ignore_ascii_case(payload.tx_hash.as_str()))
            {
                tracing::info!(
                    "Intent {} commit already reconciled with tx {}",
                    intent_id,
                    payload.tx_hash
                );
                return Ok(StatusCode::OK);
            }

            if intent.status == "AWAITING_ONCHAIN" || intent.status == "ONCHAIN_FAILED" {
                let reassigned_batch_id = compute_current_batch_id(
                    config.intents.genesis_timestamp,
                    config.intents.batch_window_seconds,
                );
                if reassigned_batch_id != intent.batch_id {
                    tracing::info!(
                        "Reassigning committed intent {} from batch {} to {}",
                        intent_id,
                        intent.batch_id,
                        reassigned_batch_id
                    );
                }
                sqlx::query!(
                    r#"
                    UPDATE intents
                    SET status = 'PENDING',
                        batch_id = $4,
                        onchain_tx_hash = $2,
                        onchain_committed_at = $3
                    WHERE intent_id = $1
                    "#,
                    intent_id,
                    payload.tx_hash,
                    now,
                    reassigned_batch_id,
                )
                .execute(&pool)
                .await
                .map_err(|e| {
                    tracing::error!("Database error: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;

                let encrypted_session_key = intent.encrypted_session_key.ok_or_else(|| {
                    tracing::error!("Missing encrypted_session_key for intent {}", intent_id);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
                let plaintext = crypto::decrypt_from_client(
                    &intent.ciphertext,
                    &encrypted_session_key,
                    &intent.client_public_key,
                    &config.security.gateway_private_key,
                )
                .map_err(|e| {
                    tracing::error!("Failed to decrypt committed intent {}: {}", intent_id, e);
                    StatusCode::BAD_REQUEST
                })?;

                crate::websocket::broadcast_new_intent(
                    &broadcaster,
                    intent_id.clone(),
                    intent.batch_id,
                    plaintext,
                )
                .await;
            } else if intent.submission_mode == "GATEWAY"
                && matches!(intent.status.as_str(), "PENDING" | "AUCTION" | "SETTLED")
                && intent.onchain_tx_hash.is_none()
            {
                sqlx::query!(
                    r#"
                    UPDATE intents
                    SET onchain_tx_hash = $2,
                        onchain_committed_at = COALESCE(onchain_committed_at, $3)
                    WHERE intent_id = $1
                    "#,
                    intent_id,
                    payload.tx_hash,
                    now,
                )
                .execute(&pool)
                .await
                .map_err(|e| {
                    tracing::error!("Database error: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
            } else {
                tracing::warn!("Intent {} is not awaiting on-chain commit", intent_id);
                return Err(StatusCode::BAD_REQUEST);
            }
        }
        "CANCELED" => {
            if intent.status == "CANCELED"
                && intent
                    .onchain_tx_hash
                    .as_deref()
                    .is_some_and(|existing| existing.eq_ignore_ascii_case(payload.tx_hash.as_str()))
            {
                tracing::info!(
                    "Intent {} cancel already reconciled with tx {}",
                    intent_id,
                    payload.tx_hash
                );
                return Ok(StatusCode::OK);
            }

            if !matches!(
                intent.status.as_str(),
                "AWAITING_ONCHAIN" | "PENDING" | "ONCHAIN_FAILED" | "AUCTION"
            ) {
                tracing::warn!("Intent {} cannot be marked canceled from {}", intent_id, intent.status);
                return Err(StatusCode::BAD_REQUEST);
            }

            let pending_amount = intent.amount.ok_or_else(|| {
                tracing::error!("Missing amount for intent {}", intent_id);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

            let mut tx = pool.begin().await.map_err(|e| {
                tracing::error!("Database error: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

            sqlx::query!(
                r#"
                UPDATE intents
                SET status = 'CANCELED',
                    onchain_tx_hash = $2,
                    canceled_at = $3
                WHERE intent_id = $1
                "#,
                intent_id,
                payload.tx_hash,
                now,
            )
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                tracing::error!("Database error: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

            sqlx::query!(
                r#"
                UPDATE pending_balances
                SET pending_amount = GREATEST(pending_amount - $3, 0)
                WHERE user_address = $1 AND asset_address = $2
                "#,
                payload.user_address,
                intent.asset_in,
                pending_amount,
            )
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                tracing::error!("Database error: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

            tx.commit().await.map_err(|e| {
                tracing::error!("Database error: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        }
        _ => {
            tracing::warn!("Unsupported on-chain lifecycle action: {}", payload.action);
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    Ok(StatusCode::OK)
}

fn compute_current_batch_id(genesis_timestamp: u64, batch_window_seconds: u64) -> String {
    let current_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_secs(0))
        .as_secs();
    
    let elapsed = current_time.saturating_sub(genesis_timestamp);
    let batch_number = elapsed / batch_window_seconds;
    
    batch_number.to_string()
}

fn compute_execution_time(
    genesis_timestamp: u64,
    batch_window_seconds: u64,
    execution_delay_seconds: u64,
) -> u64 {
    let current_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_secs(0))
        .as_secs();

    let elapsed = current_time.saturating_sub(genesis_timestamp);
    let current_batch = elapsed / batch_window_seconds;
    let next_batch_time = genesis_timestamp + ((current_batch + 1) * batch_window_seconds);

    next_batch_time + execution_delay_seconds
}

#[derive(Debug, Serialize)]
pub struct PendingIntentsResponse {
    pub count: u32,
    pub batch_id: String,
}

pub async fn get_pending_intents(
    axum::extract::State(pool): axum::extract::State<sqlx::PgPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(rate_limiter): Extension<Arc<RateLimiter>>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(params): Query<PendingQuery>,
) -> Result<Json<PendingIntentsResponse>, StatusCode> {
    require_api_key(&headers, &config)?;

    let client_ip = client_addr.ip().to_string();
    rate_limiter.check_ip_limit(&client_ip).await.map_err(|e| {
        tracing::warn!("IP rate limit exceeded: {}", e);
        StatusCode::TOO_MANY_REQUESTS
    })?;

    let batch_id = params.batch_id.unwrap_or_else(|| compute_current_batch_id(
        config.intents.genesis_timestamp,
        config.intents.batch_window_seconds
    ));

    let result = sqlx::query!(
        r#"
        SELECT COUNT(*) as count
        FROM intents
        WHERE status = 'PENDING' AND batch_id = $1
        "#,
        batch_id
    )
    .fetch_one(&pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let count = result.count.unwrap_or(0) as u32;
    
    Ok(Json(PendingIntentsResponse {
        count,
        batch_id,
    }))
}

#[derive(Debug, Deserialize)]
pub struct PendingQuery {
    pub batch_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CloseBatchRequest {
    pub batch_id: String,
    pub close_time: i64,
    pub intent_count: i32,
    pub auction_deadline: i64,
}

#[derive(Debug, Deserialize)]
pub struct SettleBatchRequest {
    pub batch_id: String,
    pub settled_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct FailedBatchRequest {
    pub batch_id: String,
    pub failure_reason: String,
    pub failed_at: i64,
}

pub async fn close_batch(
    axum::extract::State(pool): axum::extract::State<sqlx::PgPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(rate_limiter): Extension<Arc<RateLimiter>>,
    Extension(broadcaster): Extension<Arc<crate::websocket::SolverBroadcaster>>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(payload): Json<CloseBatchRequest>,
) -> Result<StatusCode, StatusCode> {
    require_api_key(&headers, &config)?;
    validate_close_batch_request(&payload)?;

    let client_ip = client_addr.ip().to_string();
    rate_limiter.check_ip_limit(&client_ip).await.map_err(|e| {
        tracing::warn!("IP rate limit exceeded: {}", e);
        StatusCode::TOO_MANY_REQUESTS
    })?;

    sqlx::query!(
        r#"
        INSERT INTO batches (batch_id, close_time, intent_count, auction_deadline, status)
        VALUES ($1, $2, $3, $4, 'AUCTION')
        ON CONFLICT (batch_id) DO UPDATE
        SET close_time = EXCLUDED.close_time,
            intent_count = EXCLUDED.intent_count,
            auction_deadline = EXCLUDED.auction_deadline,
            status = 'AUCTION'
        "#,
        payload.batch_id,
        payload.close_time,
        payload.intent_count,
        payload.auction_deadline,
    )
    .execute(&pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    sqlx::query!(
        r#"
        UPDATE intents
        SET status = 'AUCTION'
        WHERE batch_id = $1 AND status = 'PENDING'
        "#,
        payload.batch_id
    )
    .execute(&pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    crate::websocket::broadcast_batch_closed(
        &broadcaster,
        payload.batch_id,
        payload.close_time,
        payload.intent_count,
        payload.auction_deadline,
    ).await;

    metrics::init_metrics().batches_closed_total.inc();

    Ok(StatusCode::OK)
}

pub async fn fail_batch(
    axum::extract::State(pool): axum::extract::State<sqlx::PgPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(rate_limiter): Extension<Arc<RateLimiter>>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(payload): Json<FailedBatchRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    require_api_key(&headers, &config)?;
    validate_failed_batch_request(&payload)?;

    let client_ip = client_addr.ip().to_string();
    rate_limiter.check_ip_limit(&client_ip).await.map_err(|e| {
        tracing::warn!("IP rate limit exceeded: {}", e);
        StatusCode::TOO_MANY_REQUESTS
    })?;

    let requeue_batch_id = compute_current_batch_id(
        config.intents.genesis_timestamp,
        config.intents.batch_window_seconds,
    );

    let mut tx = pool.begin().await.map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    sqlx::query!(
        r#"
        UPDATE batches
        SET status = 'FAILED', failed_at = $2, failure_reason = $3
        WHERE batch_id = $1
        "#,
        payload.batch_id,
        chrono::NaiveDateTime::from_timestamp_opt(payload.failed_at, 0),
        payload.failure_reason,
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    sqlx::query!(
        r#"
        UPDATE intents
        SET batch_id = $2, status = 'PENDING'
        WHERE batch_id = $1 AND status = 'AUCTION'
        "#,
        payload.batch_id,
        requeue_batch_id,
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tx.commit().await.map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    metrics::init_metrics().batches_failed_total.inc();

    Ok(Json(serde_json::json!({
        "batch_id": payload.batch_id,
        "requeued_batch_id": requeue_batch_id
    })))
}

pub async fn settle_batch(
    axum::extract::State(pool): axum::extract::State<sqlx::PgPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(rate_limiter): Extension<Arc<RateLimiter>>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(payload): Json<SettleBatchRequest>,
) -> Result<StatusCode, StatusCode> {
    require_api_key(&headers, &config)?;
    validate_settle_batch_request(&payload)?;

    let client_ip = client_addr.ip().to_string();
    rate_limiter.check_ip_limit(&client_ip).await.map_err(|e| {
        tracing::warn!("IP rate limit exceeded: {}", e);
        StatusCode::TOO_MANY_REQUESTS
    })?;

    let mut tx = pool.begin().await.map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let intents = sqlx::query!(
        r#"
        SELECT intent_id, ciphertext, encrypted_session_key, client_public_key, user_address
        FROM intents
        WHERE batch_id = $1 AND status IN ('PENDING', 'AUCTION')
        "#,
        payload.batch_id
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    for intent in intents {
        let encrypted_session_key = intent.encrypted_session_key.as_deref().ok_or_else(|| {
            tracing::error!("Missing encrypted_session_key for intent {}", intent.intent_id);
            StatusCode::BAD_REQUEST
        })?;
        let decrypted = crypto::decrypt_from_client(
            &intent.ciphertext,
            encrypted_session_key,
            &intent.client_public_key,
            &config.security.gateway_private_key,
        ).map_err(|e| {
            tracing::error!("Failed to decrypt intent {}: {}", intent.intent_id, e);
            StatusCode::BAD_REQUEST
        })?;

        let decrypted_intent: DecryptedIntent = serde_json::from_slice(&decrypted).map_err(|e| {
            tracing::error!("Failed to parse decrypted intent {}: {}", intent.intent_id, e);
            StatusCode::BAD_REQUEST
        })?;

        let pending_amount = BigDecimal::from_str(&decrypted_intent.amount.to_string()).map_err(|e| {
            tracing::error!("Invalid amount format: {}", e);
            StatusCode::BAD_REQUEST
        })?;

        sqlx::query!(
            r#"
            UPDATE pending_balances
            SET pending_amount = GREATEST(pending_amount - $3, 0)
            WHERE user_address = $1 AND asset_address = $2
            "#,
            intent.user_address,
            decrypted_intent.asset_in,
            pending_amount,
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!("Database error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }

    sqlx::query!(
        r#"
        UPDATE intents
        SET status = 'SETTLED'
        WHERE batch_id = $1 AND status IN ('PENDING', 'AUCTION')
        "#,
        payload.batch_id
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    sqlx::query!(
        r#"
        UPDATE batches
        SET status = 'SETTLED', settled_at = $2
        WHERE batch_id = $1
        "#,
        payload.batch_id,
        chrono::NaiveDateTime::from_timestamp_opt(payload.settled_at, 0)
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    sqlx::query!(
        r#"
        UPDATE pending_balances AS pb
        SET pending_amount = GREATEST(pb.pending_amount - sub.total_amount, 0)
        FROM (
            SELECT user_address, asset_in, SUM(amount) AS total_amount
            FROM intents
            WHERE batch_id = $1 AND status = 'SETTLED'
            GROUP BY user_address, asset_in
        ) AS sub
        WHERE pb.user_address = sub.user_address AND pb.asset_address = sub.asset_in
        "#,
        payload.batch_id
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tx.commit().await.map_err(|e| {
        tracing::error!("Database error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    metrics::init_metrics().batches_settled_total.inc();

    Ok(StatusCode::OK)
}

pub async fn get_gateway_public_key(
    Extension(rate_limiter): Extension<Arc<RateLimiter>>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Extension(config): Extension<Arc<Config>>,
) -> Result<Json<GatewayPublicKeyResponse>, StatusCode> {
    require_api_key(&headers, &config)?;

    let client_ip = client_addr.ip().to_string();
    rate_limiter.check_ip_limit(&client_ip).await.map_err(|e| {
        tracing::warn!("IP rate limit exceeded: {}", e);
        StatusCode::TOO_MANY_REQUESTS
    })?;

    let gateway_public_key = crypto::public_key_from_private(&config.security.gateway_private_key)
        .map_err(|e| {
            tracing::error!("Failed to derive gateway public key: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(GatewayPublicKeyResponse { gateway_public_key }))
}

fn extract_user_address_from_intent(payload: &SubmitIntentRequest) -> Result<String, StatusCode> {
    if payload.user_address.is_empty() {
        tracing::error!("Missing user_address in request");
        return Err(StatusCode::BAD_REQUEST);
    }

    if !payload.user_address.starts_with("0x") {
        tracing::error!("Invalid user_address format: must start with 0x");
        return Err(StatusCode::BAD_REQUEST);
    }

    Ok(payload.user_address.clone())
}

fn require_api_key(headers: &HeaderMap, config: &Config) -> Result<(), StatusCode> {
    let header_name = config.security.api_key_header.as_str();
    let provided = headers.get(header_name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");

    if provided.is_empty() {
        tracing::warn!("Missing API key header: {}", header_name);
        return Err(StatusCode::UNAUTHORIZED);
    }

    if provided != config.security.api_key {
        tracing::warn!("Invalid API key");
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(())
}

fn validate_submit_request(payload: &SubmitIntentRequest) -> Result<(), StatusCode> {
    if payload.intent_id.is_empty() || payload.intent_id.len() > 66 {
        tracing::warn!("Invalid intent_id length");
        return Err(StatusCode::BAD_REQUEST);
    }

    if !payload.intent_id.starts_with("0x") {
        tracing::warn!("intent_id must be hex with 0x prefix");
        return Err(StatusCode::BAD_REQUEST);
    }

    if payload.commitment.is_empty() || !payload.commitment.starts_with("0x") {
        tracing::warn!("commitment must be hex with 0x prefix");
        return Err(StatusCode::BAD_REQUEST);
    }

    if payload.ciphertext.is_empty() {
        tracing::warn!("ciphertext is required");
        return Err(StatusCode::BAD_REQUEST);
    }

    if !is_hex_bytes(&payload.ciphertext) {
        tracing::warn!("ciphertext must be hex with 0x prefix");
        return Err(StatusCode::BAD_REQUEST);
    }

    if payload.client_public_key.is_empty() || !payload.client_public_key.starts_with("0x") {
        tracing::warn!("client_public_key must be hex with 0x prefix");
        return Err(StatusCode::BAD_REQUEST);
    }

    if payload.encrypted_session_key.is_empty() || !payload.encrypted_session_key.starts_with("0x") {
        tracing::warn!("encrypted_session_key must be hex with 0x prefix");
        return Err(StatusCode::BAD_REQUEST);
    }
    if !is_hex_bytes(&payload.encrypted_session_key) {
        tracing::warn!("encrypted_session_key must be valid hex bytes");
        return Err(StatusCode::BAD_REQUEST);
    }

    if !is_hex_bytes(&payload.client_public_key) {
        tracing::warn!("client_public_key must be valid hex");
        return Err(StatusCode::BAD_REQUEST);
    }

    if payload.client_public_key.len() != 66 {
        tracing::warn!("client_public_key must be 32 bytes");
        return Err(StatusCode::BAD_REQUEST);
    }

    if payload.user_signature.len() < 2 {
        tracing::warn!("user_signature must include r and s");
        return Err(StatusCode::BAD_REQUEST);
    }

    if let Some(ref mode) = payload.submission_mode {
        let normalized = mode.trim().to_ascii_uppercase();
        if normalized != "GATEWAY" && normalized != "SELF_COMMIT" {
            tracing::warn!("submission_mode must be GATEWAY or SELF_COMMIT");
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    // Lightweight hex check for fields that must be field elements
    // encrypted_session_key is binary data (60 bytes), already validated with is_hex_bytes above.
    // Only check field-element-sized values here.
    for (label, value) in [
        ("intent_id", payload.intent_id.as_str()),
        ("commitment", payload.commitment.as_str()),
        ("client_public_key", payload.client_public_key.as_str()),
        ("signature_r", payload.user_signature[0].as_str()),
        ("signature_s", payload.user_signature[1].as_str()),
        ("nonce", payload.nonce.as_str()),
    ] {
        if !is_hex_felt(value) {
            tracing::warn!("Invalid hex for {}", label);
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    if let Some(ref authorization_hash) = payload.authorization_hash {
        if !is_hex_felt(authorization_hash) {
            tracing::warn!("authorization_hash must be valid felt hex");
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    Ok(())
}

fn validate_close_batch_request(payload: &CloseBatchRequest) -> Result<(), StatusCode> {
    if payload.batch_id.is_empty() || !payload.batch_id.chars().all(|c| c.is_ascii_digit()) {
        tracing::warn!("batch_id must be numeric");
        return Err(StatusCode::BAD_REQUEST);
    }

    if payload.close_time <= 0 || payload.auction_deadline <= 0 {
        tracing::warn!("close_time and auction_deadline must be positive");
        return Err(StatusCode::BAD_REQUEST);
    }

    if payload.intent_count < 0 {
        tracing::warn!("intent_count must be non-negative");
        return Err(StatusCode::BAD_REQUEST);
    }

    Ok(())
}

fn validate_settle_batch_request(payload: &SettleBatchRequest) -> Result<(), StatusCode> {
    if payload.batch_id.is_empty() || !payload.batch_id.chars().all(|c| c.is_ascii_digit()) {
        tracing::warn!("batch_id must be numeric");
        return Err(StatusCode::BAD_REQUEST);
    }

    if payload.settled_at <= 0 {
        tracing::warn!("settled_at must be positive");
        return Err(StatusCode::BAD_REQUEST);
    }

    Ok(())
}

fn validate_failed_batch_request(payload: &FailedBatchRequest) -> Result<(), StatusCode> {
    if payload.batch_id.is_empty() || !payload.batch_id.chars().all(|c| c.is_ascii_digit()) {
        tracing::warn!("batch_id must be numeric");
        return Err(StatusCode::BAD_REQUEST);
    }

    if payload.failed_at <= 0 {
        tracing::warn!("failed_at must be positive");
        return Err(StatusCode::BAD_REQUEST);
    }

    if payload.failure_reason.is_empty() || !payload.failure_reason.starts_with("0x") {
        tracing::warn!("failure_reason must be hex with 0x prefix");
        return Err(StatusCode::BAD_REQUEST);
    }
    if !is_hex_felt(&payload.failure_reason) {
        tracing::warn!("failure_reason must be valid felt hex");
        return Err(StatusCode::BAD_REQUEST);
    }

    Ok(())
}

fn validate_onchain_lifecycle_request(payload: &OnchainLifecycleRequest) -> Result<(), StatusCode> {
    let action = payload.action.trim().to_ascii_uppercase();
    if action != "COMMITTED" && action != "CANCELED" {
        tracing::warn!("action must be COMMITTED or CANCELED");
        return Err(StatusCode::BAD_REQUEST);
    }

    if !is_hex_felt(&payload.user_address) {
        tracing::warn!("user_address must be valid felt hex");
        return Err(StatusCode::BAD_REQUEST);
    }

    if !is_hex_felt(&payload.tx_hash) {
        tracing::warn!("tx_hash must be valid felt hex");
        return Err(StatusCode::BAD_REQUEST);
    }

    Ok(())
}

fn parse_submission_mode(value: Option<&str>) -> Result<SubmissionMode, StatusCode> {
    match value.map(|mode| mode.trim().to_ascii_uppercase()) {
        None => Ok(SubmissionMode::Gateway),
        Some(mode) if mode == "GATEWAY" => Ok(SubmissionMode::Gateway),
        Some(mode) if mode == "SELF_COMMIT" => Ok(SubmissionMode::SelfCommit),
        Some(_) => Err(StatusCode::BAD_REQUEST),
    }
}

fn deserialize_u128<'de, D>(deserializer: D) -> Result<u128, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(num) => num.as_u64()
            .map(|v| v as u128)
            .ok_or_else(|| serde::de::Error::custom("Invalid number for u128")),
        serde_json::Value::String(text) => text.parse::<u128>()
            .map_err(|_| serde::de::Error::custom("Invalid string for u128")),
        _ => Err(serde::de::Error::custom("Invalid type for u128")),
    }
}

fn deserialize_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(num) => num
            .as_u64()
            .ok_or_else(|| serde::de::Error::custom("Invalid number for u64")),
        serde_json::Value::String(text) => text
            .parse::<u64>()
            .map_err(|_| serde::de::Error::custom("Invalid string for u64")),
        _ => Err(serde::de::Error::custom("Invalid type for u64")),
    }
}

fn field_element_from_hex(value: &str, label: &str) -> Result<Felt, StatusCode> {
    if !value.starts_with("0x") {
        tracing::warn!("{} must start with 0x", label);
        return Err(StatusCode::BAD_REQUEST);
    }
    Felt::from_hex(value)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

fn compute_amount_commitment(amount: u128, nonce_hex: &str) -> Result<Felt, StatusCode> {
    let amount_fe = Felt::from(amount);
    let nonce_fe = field_element_from_hex(nonce_hex, "nonce")?;
    Ok(pedersen_hash(&amount_fe, &nonce_fe))
}

fn compute_intent_hash(
    user_address_hex: &str,
    asset_in_hex: &str,
    asset_out_hex: &str,
    amount_commitment: Felt,
    min_output: u128,
    max_fee_bps: u16,
    deadline: u64,
    privacy_mode: u8,
    nonce_hex: &str,
) -> Result<Felt, StatusCode> {
    let user = field_element_from_hex(user_address_hex, "user_address")?;
    let asset_in = field_element_from_hex(asset_in_hex, "asset_in")?;
    let asset_out = field_element_from_hex(asset_out_hex, "asset_out")?;
    let nonce = field_element_from_hex(nonce_hex, "nonce")?;

    let mut hash = pedersen_hash(&user, &asset_in);
    hash = pedersen_hash(&hash, &asset_out);
    hash = pedersen_hash(&hash, &amount_commitment);

    let min_low = Felt::from(min_output);
    let min_high = Felt::ZERO;
    let min_output_hash = pedersen_hash(&min_low, &min_high);
    hash = pedersen_hash(&hash, &min_output_hash);

    let max_fee = Felt::from(max_fee_bps as u128);
    hash = pedersen_hash(&hash, &max_fee);
    let deadline_fe = Felt::from(deadline as u128);
    hash = pedersen_hash(&hash, &deadline_fe);
    let privacy = Felt::from(privacy_mode as u128);
    hash = pedersen_hash(&hash, &privacy);
    hash = pedersen_hash(&hash, &nonce);

    Ok(hash)
}

fn compute_intent_id(
    user_address_hex: &str,
    nonce_hex: &str,
    intent_hash: Felt,
) -> Result<Felt, StatusCode> {
    let user = field_element_from_hex(user_address_hex, "user_address")?;
    let nonce = field_element_from_hex(nonce_hex, "nonce")?;
    let inner = pedersen_hash(&user, &nonce);
    Ok(pedersen_hash(&inner, &intent_hash))
}

fn is_hex_felt(value: &str) -> bool {
    if !value.starts_with("0x") {
        return false;
    }
    let hex = &value[2..];
    !hex.is_empty() && hex.len() <= 64 && hex.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_hex_bytes(value: &str) -> bool {
    if !value.starts_with("0x") {
        return false;
    }
    let hex = &value[2..];
    !hex.is_empty() && hex.len() % 2 == 0 && hex.chars().all(|c| c.is_ascii_hexdigit())
}
