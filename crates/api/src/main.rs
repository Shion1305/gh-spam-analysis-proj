use anyhow::Result;
use axum::{routing::get, Router};
use common::{config::AppConfig, logging};
use std::net::SocketAddr;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    logging::init_logging("info");
    let config = AppConfig::load()?;

    let app = Router::new().route("/healthz", get(|| async { "ok" }));

    let addr: SocketAddr = config.api.bind.parse()?;
    info!("api listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;
    Ok(())
}
