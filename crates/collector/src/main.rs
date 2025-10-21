use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use collector::{BrokerGithubClient, Collector};
use common::{config::AppConfig, logging};
use db::pg::{run_migrations, PgDatabase};
use db::Repositories;
use gh_broker::{Budget, GithubBrokerBuilder, GithubToken as BrokerToken, Priority};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    logging::init_logging("info");
    let config = AppConfig::load()?;
    let tokens = config.github.resolved_tokens()?;
    if tokens.is_empty() {
        return Err(anyhow!("no GitHub tokens configured"));
    }

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

    let broker = builder.build();
    let client = Arc::new(BrokerGithubClient::new(
        broker,
        config.github.user_agent.clone(),
    ));

    let database = Arc::new(PgDatabase::connect(&config.database.url).await?);
    run_migrations(database.pool()).await?;
    let repositories: Arc<dyn Repositories> = database.clone() as Arc<dyn Repositories>;

    let collector = Collector::new(config.collector.clone(), client, repositories);
    info!(
        interval = config.collector.interval_secs,
        "collector started"
    );
    collector.run().await?;
    Ok(())
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
