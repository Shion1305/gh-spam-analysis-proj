use anyhow::Result;
use common::{config::AppConfig, logging};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    logging::init_logging("info");
    let config = AppConfig::load()?;
    info!(
        "collector starting with interval {}s",
        config.collector.interval_secs
    );
    Ok(())
}
