use axum::{
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber;
use sqlx::postgres::PgPoolOptions;
use axum::http::{Method, HeaderValue};

mod api;
mod auth;
mod ciphertext_pool;
mod rate_limiter;
mod pending_ledger;
mod config;
mod websocket;
mod starknet_client;
mod crypto;
mod metrics;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = config::Config::from_env()
        .map_err(|e| anyhow::anyhow!("Failed to load configuration: {}", e))?;

    config.validate()
        .map_err(|e| anyhow::anyhow!("Invalid configuration: {}", e))?;

    tracing::info!("Configuration loaded successfully");

    let db_pool = PgPoolOptions::new()
        .max_connections(config.database.max_connections)
        .connect(&config.database.url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to database: {}", e))?;

    tracing::info!("Database connection established");

    sqlx::migrate!("./migrations")
        .run(&db_pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to run migrations: {}", e))?;

    tracing::info!("Database migrations completed");

    // Create WebSocket broadcaster for solver notifications
    let broadcaster = Arc::new(websocket::create_broadcaster(
        config.server.websocket_channel_capacity
    ));
    let redis_client = redis::Client::open(config.redis.url.as_str())
        .map_err(|e| anyhow::anyhow!("Failed to create Redis client: {}", e))?;
    let redis_conn = redis_client
        .get_tokio_connection_manager()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to Redis: {}", e))?;
    let rate_limiter = Arc::new(rate_limiter::RateLimiter::new(
        redis_conn,
        config.rate_limit.max_intents_per_user_per_minute,
        config.rate_limit.max_requests_per_ip_per_minute,
        config.rate_limit.window_seconds,
    ));
    let config = Arc::new(config);
    metrics::init_metrics();

    let starknet_client = Arc::new(
        starknet_client::StarknetClient::new(
            starknet_client::StarknetConfig::from_config(&config)
        ).map_err(|e| anyhow::anyhow!("Failed to initialize Starknet client: {}", e))?
    );

    let cors = build_cors_layer(&config)?;

    let app = Router::new()
        .route("/health", get(health_check))
        .route("/metrics", get(metrics_handler))
        .route("/v1/intents", post(api::submit_intent).get(api::list_intents))
        .route("/v1/intents/pending", get(api::get_pending_intents))
        .route("/v1/intents/:intent_id", get(api::get_intent_status))
        .route("/v1/intents/:intent_id/cancel", post(api::cancel_intent))
        .route("/v1/intents/:intent_id/onchain", post(api::reconcile_onchain_intent))
        .route("/v1/batches/:batch_id/intents", get(api::list_batch_intents))
        .route("/v1/batches/:batch_id/solver_intents", get(api::get_solver_batch_intents))
        .route("/v1/gateway/public_key", get(api::get_gateway_public_key))
        .route("/v1/batches/close", post(api::close_batch))
        .route("/v1/batches/failed", post(api::fail_batch))
        .route("/v1/batches/settled", post(api::settle_batch))
        .route("/v1/batches", get(api::list_batches))
        .route("/v1/solver/ws", get(websocket::solver_websocket_handler))
        .with_state(db_pool.clone())
        .layer(axum::Extension(broadcaster))
        .layer(axum::Extension(rate_limiter))
        .layer(axum::Extension(config.clone()))
        .layer(axum::Extension(starknet_client))
        .layer(cors);

    let addr = SocketAddr::from((
        config.server.host.parse::<std::net::IpAddr>()?,
        config.server.port
    ));

    tracing::info!("Gateway listening on {}", addr);
    tracing::info!("WebSocket endpoint available at ws://{}/v1/solver/ws", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;

    Ok(())
}

async fn health_check() -> &'static str {
    "OK"
}

async fn metrics_handler() -> Result<String, axum::http::StatusCode> {
    metrics::gather_metrics()
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)
}

fn build_cors_layer(config: &config::Config) -> Result<CorsLayer, anyhow::Error> {
    let origins = config.security.cors_allowed_origins
        .split(',')
        .map(|origin| origin.trim())
        .filter(|origin| !origin.is_empty())
        .map(|origin| origin.parse::<HeaderValue>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("Invalid CORS_ALLOWED_ORIGINS entry: {}", e))?;

    if origins.is_empty() {
        return Err(anyhow::anyhow!("CORS_ALLOWED_ORIGINS must contain at least one origin"));
    }

    let api_key_header = axum::http::header::HeaderName::from_bytes(
        config.security.api_key_header.as_bytes()
    )
    .map_err(|e| anyhow::anyhow!("Invalid API_KEY_HEADER: {}", e))?;

    Ok(CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            api_key_header,
        ]))
}
