use std::path::Path;

use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub database: DatabaseConfig,
    pub github: GithubConfig,
    pub collector: CollectorConfig,
    pub broker: BrokerConfig,
    pub api: ApiConfig,
    pub observability: ObservabilityConfig,
}

impl AppConfig {
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from_path(".")
    }

    pub fn load_from_path<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        dotenvy::dotenv().ok();

        Config::builder()
            .add_source(
                File::with_name(
                    path.as_ref()
                        .join("config/default")
                        .to_string_lossy()
                        .as_ref(),
                )
                .required(false),
            )
            .add_source(
                File::with_name(
                    path.as_ref()
                        .join("config/local")
                        .to_string_lossy()
                        .as_ref(),
                )
                .required(false),
            )
            .add_source(Environment::default().separator("__"))
            .build()?
            .try_deserialize()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    #[serde(default)]
    pub test_admin_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubToken {
    pub id: String,
    pub secret: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubConfig {
    pub tokens: Vec<GithubToken>,
    pub user_agent: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CollectorConfig {
    #[serde(default = "CollectorConfig::default_interval_secs")]
    pub interval_secs: u64,
    #[serde(default = "CollectorConfig::default_page_size")]
    pub page_size: u32,
    #[serde(default)]
    pub run_once: bool,
}

impl CollectorConfig {
    const fn default_interval_secs() -> u64 {
        300
    }

    const fn default_page_size() -> u32 {
        100
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BrokerConfig {
    #[serde(default = "BrokerConfig::default_max_inflight")]
    pub max_inflight: usize,
    #[serde(default = "BrokerConfig::default_per_repo_inflight")]
    pub per_repo_inflight: usize,
    #[serde(default)]
    pub distributed: bool,
}

impl BrokerConfig {
    const fn default_max_inflight() -> usize {
        32
    }

    const fn default_per_repo_inflight() -> usize {
        2
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiConfig {
    pub bind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ObservabilityConfig {
    #[serde(default = "ObservabilityConfig::default_metrics_path")]
    pub metrics_path: String,
}

impl ObservabilityConfig {
    fn default_metrics_path() -> String {
        "/metrics".to_string()
    }
}
