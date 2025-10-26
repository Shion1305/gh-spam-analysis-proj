use std::sync::Arc;

use anyhow::Result;
use api::{build_router, ApiState};
use axum::Router;
use common::{config::AppConfig, logging};
use db::pg::PgDatabase;
use db::Repositories;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    logging::init_tracing("api", "info");
    let config = AppConfig::load()?;
    let database = Arc::new(PgDatabase::connect(&config.database.url).await?);
    let repositories: Arc<dyn Repositories> = database.clone();
    let metrics_path: &'static str =
        Box::leak(config.observability.metrics_path.clone().into_boxed_str());
    let state = Arc::new(ApiState {
        repositories,
        metrics_path,
        pool: Arc::new(database.pool().clone()),
    });
    let app: Router = build_router(state);

    let addr: std::net::SocketAddr = config.api.bind.parse()?;
    info!("api listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    // Ensure any remaining spans are flushed on shutdown (no-op if otel disabled)
    common::logging::shutdown_tracer_provider();
    Ok(())
}
