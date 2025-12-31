use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use axum::http::header;
use axum::http::StatusCode;
use axum::response::Response;
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
use serde_json::Value;
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

    if let Err(err) = encoder.encode(&metric_families, &mut buffer) {
        warn!(error = ?err, "failed to encode collector metrics");
        let mut res = Response::new(axum::body::Body::from("failed to encode metrics"));
        *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        res.headers_mut()
            .insert(header::CONTENT_TYPE, "text/plain".parse().unwrap());
        return res;
    }

    let content_type = encoder.format_type().to_string();
    let mut res = Response::new(axum::body::Body::from(buffer));
    *res.status_mut() = StatusCode::OK;
    res.headers_mut()
        .insert(header::CONTENT_TYPE, content_type.parse().unwrap());
    res
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
    let config = match AppConfig::load() {
        Ok(c) => c,
        Err(_err) => {
            return Json(RateLimitsResponse {
                tokens: vec![TokenRateLimit {
                    token: "<config-error>".to_string(),
                    budget: "n/a".to_string(),
                    limit: 0,
                    remaining: 0,
                }],
            })
        }
    };

    let tokens = match config.github.resolved_tokens() {
        Ok(t) => t,
        Err(_err) => {
            return Json(RateLimitsResponse {
                tokens: vec![TokenRateLimit {
                    token: "<config-error>".to_string(),
                    budget: "n/a".to_string(),
                    limit: 0,
                    remaining: 0,
                }],
            })
        }
    };

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

    let client = match builder.build() {
        Ok(c) => c,
        Err(_) => {
            return Json(RateLimitsResponse { tokens: Vec::new() });
        }
    };

    let mut out = Vec::new();

    for token in tokens {
        let resp = match client
            .get("https://api.github.com/rate_limit")
            .bearer_auth(&token.secret)
            .send()
            .await
        {
            Ok(r) => r,
            Err(_err) => continue,
        };

        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            // Surface non-success statuses as synthetic entries per token.
            out.push(TokenRateLimit {
                token: token.id.clone(),
                budget: format!("status {}", status.as_u16()),
                limit: 0,
                remaining: 0,
            });
            continue;
        }

        let parsed: Value = match serde_json::from_str(&body_text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let Some(resources) = parsed.get("resources") {
            for budget_key in ["core", "graphql", "search"] {
                if let Some(b) = resources.get(budget_key) {
                    let limit = b.get("limit").and_then(|v| v.as_i64()).unwrap_or(0);
                    let remaining = b.get("remaining").and_then(|v| v.as_i64()).unwrap_or(0);
                    out.push(TokenRateLimit {
                        token: token.id.clone(),
                        budget: budget_key.to_string(),
                        limit,
                        remaining,
                    });
                }
            }
        }
    }

    Json(RateLimitsResponse { tokens: out })
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
