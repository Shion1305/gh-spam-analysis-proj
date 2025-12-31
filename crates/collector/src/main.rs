use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use collector::{
    fetcher::{DataFetcher, GraphqlDataFetcher, RestDataFetcher},
    BrokerGithubClient, Collector, GithubClient,
};
use common::{
    config::{AppConfig, FetchMode, GithubToken},
    logging,
};
use db::pg::PgDatabase;
use db::Repositories;
use gh_broker::{Budget, GithubBrokerBuilder, GithubToken as BrokerToken, Priority};
use prometheus::Encoder;
use serde::Serialize;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    logging::init_tracing("collector", "info");
    let config = AppConfig::load()?;
    let tokens = config.github.resolved_tokens()?;
    if tokens.is_empty() {
        return Err(anyhow!("no GitHub tokens configured"));
    }

    info!(
        token_ids = ?tokens.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
        user_agent = %config.github.user_agent,
        "GitHub tokens configured for collector"
    );

    verify_github_tokens(&config, &tokens).await?;

    let broker_tokens: Vec<BrokerToken> = tokens
        .into_iter()
        .map(|token| BrokerToken {
            id: token.id,
            secret: token.secret,
        })
        .collect();

    let mut builder = GithubBrokerBuilder::new(broker_tokens)
        .max_inflight(config.broker.max_inflight)
        .per_repo_inflight(config.broker.per_repo_inflight)
        .cache(
            config.broker.cache_capacity,
            Duration::from_secs(config.broker.cache_ttl_secs),
        )
        .backoff(
            Duration::from_millis(config.broker.backoff_base_ms),
            Duration::from_millis(config.broker.backoff_max_ms),
            config.broker.jitter_frac,
        );

    if !config.broker.queue_bounds.is_empty() {
        builder = builder.queue_bounds(map_queue_bounds(&config.broker.queue_bounds));
    }
    if !config.broker.weights.is_empty() {
        builder = builder.weights(map_weights(&config.broker.weights));
    }

    let metrics_path: &'static str =
        Box::leak(config.observability.metrics_path.clone().into_boxed_str());
    let metrics_addr: SocketAddr = config.observability.metrics_bind.parse()?;
    tokio::spawn(async move {
        if let Err(err) = serve_metrics(metrics_addr, metrics_path).await {
            warn!(error = ?err, "collector metrics server exited");
        }
    });

    let broker = builder.build();
    let client: Arc<dyn GithubClient> = Arc::new(BrokerGithubClient::new(
        broker.clone(),
        config.github.user_agent.clone(),
    ));
    let fetcher: Arc<dyn DataFetcher> = match config.collector.fetch_mode {
        FetchMode::Rest => Arc::new(RestDataFetcher::new(client.clone())),
        FetchMode::Graphql => Arc::new(GraphqlDataFetcher::new(
            broker.clone(),
            client.clone(),
            config.github.user_agent.clone(),
        )),
        FetchMode::Hybrid => Arc::new(collector::fetcher::HybridDataFetcher::new(
            broker.clone(),
            client.clone(),
            config.github.user_agent.clone(),
        )),
    };
    info!(fetch_mode = ?config.collector.fetch_mode, "collector fetch mode selected");

    let database = Arc::new(PgDatabase::connect(&config.database.url).await?);
    let repositories: Arc<dyn Repositories> = database.clone() as Arc<dyn Repositories>;

    let max_repos = std::env::var("COLLECTOR__MAX_CONCURRENT_REPOS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(4);
    let collector = Collector::new(config.collector.clone(), fetcher, repositories, max_repos);
    info!(
        interval = config.collector.interval_secs,
        "collector started"
    );
    collector.run().await?;
    // Ensure any remaining spans are flushed on shutdown (no-op if otel disabled)
    common::logging::shutdown_tracer_provider();
    Ok(())
}

async fn verify_github_tokens(config: &AppConfig, tokens: &[GithubToken]) -> Result<()> {
    use http::header;
    use http::StatusCode;

    let mut builder = reqwest::Client::builder().user_agent(config.github.user_agent.clone());

    if let Ok(proxy) = std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("https_proxy"))
        .or_else(|_| std::env::var("HTTP_PROXY"))
        .or_else(|_| std::env::var("http_proxy"))
    {
        if let Ok(p) = reqwest::Proxy::all(&proxy) {
            builder = builder.proxy(p);
        }
    }

    let client = builder.build()?;

    let mut valid_count = 0usize;
    for token in tokens {
        let resp = client
            .get("https://api.github.com/rate_limit")
            .header(header::AUTHORIZATION, format!("token {}", token.secret))
            .send()
            .await?;

        let status = resp.status();
        if status.is_success() {
            valid_count += 1;
            info!(
                token_id = %token.id,
                status = %status,
                "GitHub token verified successfully"
            );
            continue;
        }

        let body = resp.text().await.unwrap_or_default();

        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            warn!(
                token_id = %token.id,
                status = %status,
                body_preview = %body.chars().take(256).collect::<String>(),
                "GitHub token failed verification with auth error"
            );
            return Err(anyhow!(
                "GitHub token {} failed verification with status {}",
                token.id,
                status
            ));
        }

        warn!(
            token_id = %token.id,
            status = %status,
            body_preview = %body.chars().take(256).collect::<String>(),
            "GitHub token verification returned non-success status"
        );
    }

    if valid_count == 0 {
        return Err(anyhow!(
            "no valid GitHub tokens after verification; check scopes and configuration"
        ));
    }

    Ok(())
}

async fn serve_metrics(addr: SocketAddr, metrics_path: &'static str) -> Result<()> {
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route(metrics_path, get(export_metrics))
        .route("/rate_limits", get(rate_limits));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(
        address = %addr,
        path = metrics_path,
        "collector metrics server listening"
    );
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> &'static str {
    "ok"
}

async fn export_metrics() -> Response {
    let encoder = prometheus::TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();

    match encoder.encode(&metric_families, &mut buffer) {
        Ok(()) => {
            let content_type = encoder.format_type().to_string();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, content_type)],
                buffer,
            )
                .into_response()
        }
        Err(err) => {
            warn!(error = ?err, "failed to encode collector metrics");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "text/plain".to_string())],
                b"failed to encode metrics".to_vec(),
            )
                .into_response()
        }
    }
}

#[derive(Debug, Serialize)]
struct TokenRateLimit {
    token: String,
    budget: String,
    limit: i64,
    remaining: i64,
}

#[derive(Debug, Serialize)]
struct RateLimitsResponse {
    tokens: Vec<TokenRateLimit>,
}

async fn rate_limits() -> Json<RateLimitsResponse> {
    use std::collections::HashMap;

    let metric_families = prometheus::gather();

    // Map (token, budget) -> (limit, remaining)
    let mut limits: HashMap<(String, String), (i64, i64)> = HashMap::new();

    for mf in &metric_families {
        match mf.get_name() {
            "gh_broker_rate_limit" => {
                for m in mf.get_metric() {
                    let mut token = String::new();
                    let mut budget = String::new();
                    for label in m.get_label() {
                        match label.get_name() {
                            "token" => token = label.get_value().to_string(),
                            "budget" => budget = label.get_value().to_string(),
                            _ => {}
                        }
                    }
                    if !token.is_empty() && !budget.is_empty() {
                        let limit = m.get_gauge().get_value() as i64;
                        let entry = limits.entry((token, budget)).or_insert((0, 0));
                        entry.0 = limit;
                    }
                }
            }
            "gh_broker_rate_remaining" => {
                for m in mf.get_metric() {
                    let mut token = String::new();
                    let mut budget = String::new();
                    for label in m.get_label() {
                        match label.get_name() {
                            "token" => token = label.get_value().to_string(),
                            "budget" => budget = label.get_value().to_string(),
                            _ => {}
                        }
                    }
                    if !token.is_empty() && !budget.is_empty() {
                        let remaining = m.get_gauge().get_value() as i64;
                        let entry = limits.entry((token, budget)).or_insert((0, 0));
                        entry.1 = remaining;
                    }
                }
            }
            _ => {}
        }
    }

    let tokens = limits
        .into_iter()
        .map(|((token, budget), (limit, remaining))| TokenRateLimit {
            token,
            budget,
            limit,
            remaining,
        })
        .collect();

    Json(RateLimitsResponse { tokens })
}

fn map_queue_bounds(bounds: &HashMap<String, usize>) -> HashMap<(Budget, Priority), usize> {
    let mut mapped = HashMap::new();
    for (key, value) in bounds {
        if let Some((budget, priority)) = parse_queue_key(key) {
            mapped.insert((budget, priority), *value);
        }
    }
    mapped
}

fn map_weights(weights: &HashMap<String, [u32; 3]>) -> HashMap<Budget, [u32; 3]> {
    let mut mapped = HashMap::new();
    for (key, value) in weights {
        if let Some(budget) = parse_budget(key) {
            mapped.insert(budget, *value);
        }
    }
    mapped
}

fn parse_queue_key(key: &str) -> Option<(Budget, Priority)> {
    let mut parts = key.split('.');
    let budget = parse_budget(parts.next()?)?;
    let priority = parse_priority(parts.next().unwrap_or("normal"))?;
    Some((budget, priority))
}

fn parse_budget(input: &str) -> Option<Budget> {
    match input.to_ascii_lowercase().as_str() {
        "core" => Some(Budget::Core),
        "search" => Some(Budget::Search),
        "graphql" => Some(Budget::Graphql),
        _ => None,
    }
}

fn parse_priority(input: &str) -> Option<Priority> {
    match input.to_ascii_lowercase().as_str() {
        "critical" => Some(Priority::Critical),
        "normal" => Some(Priority::Normal),
        "backfill" => Some(Priority::Backfill),
        _ => None,
    }
}
