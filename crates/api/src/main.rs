use std::sync::Arc;

use anyhow::Result;
use api::{build_router, ApiState};
use axum::Router;
use common::{config::AppConfig, logging};
use db::pg::{run_migrations, PgDatabase};
use db::Repositories;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    logging::init_logging("info");
    let config = AppConfig::load()?;
    let database = Arc::new(PgDatabase::connect(&config.database.url).await?);
    run_migrations(database.pool()).await?;
    let repositories: Arc<dyn Repositories> = database.clone();
    let metrics_path: &'static str =
        Box::leak(config.observability.metrics_path.clone().into_boxed_str());
    let state = Arc::new(ApiState {
        repositories,
        metrics_path,
    });
    let app: Router = build_router(state);

    let addr = config.api.bind.parse()?;
    info!("api listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;
    Ok(())
}
