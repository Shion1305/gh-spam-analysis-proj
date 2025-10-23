use std::path::Path;

use std::collections::HashMap;

use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use serde_with::{formats::CommaSeparator, serde_as, StringWithSeparator};

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

#[serde_as]
#[derive(Debug, Clone, Deserialize)]
pub struct GithubConfig {
    #[serde(default)]
    pub tokens: Vec<GithubToken>,
    #[serde_as(as = "StringWithSeparator::<CommaSeparator, String>")]
    #[serde(default)]
    pub token_ids: Vec<String>,
    #[serde_as(as = "StringWithSeparator::<CommaSeparator, String>")]
    #[serde(default)]
    pub token_secrets: Vec<String>,
    pub user_agent: String,
}

impl GithubConfig {
    pub fn resolved_tokens(&self) -> Result<Vec<GithubToken>, ConfigError> {
        if !self.tokens.is_empty() {
            return Ok(self.tokens.clone());
        }
        if self.token_ids.len() != self.token_secrets.len() {
            return Err(ConfigError::Message(
                "GITHUB_TOKENS and GITHUB_TOKEN_SECRETS length mismatch".into(),
            ));
        }
        Ok(self
            .token_ids
            .iter()
            .cloned()
            .zip(self.token_secrets.iter().cloned())
            .map(|(id, secret)| GithubToken { id, secret })
            .collect())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CollectorConfig {
    #[serde(default = "CollectorConfig::default_interval_secs")]
    pub interval_secs: u64,
    #[serde(default = "CollectorConfig::default_page_size")]
    pub page_size: u32,
    #[serde(default)]
    pub run_once: bool,
    #[serde(default)]
    pub fetch_mode: FetchMode,
}

impl CollectorConfig {
    const fn default_interval_secs() -> u64 {
        300
    }

    const fn default_page_size() -> u32 {
        100
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FetchMode {
    Rest,
    Graphql,
    Hybrid,
}

impl Default for FetchMode {
    fn default() -> Self {
        Self::Rest
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
    #[serde(default = "BrokerConfig::default_cache_capacity")]
    pub cache_capacity: usize,
    #[serde(default = "BrokerConfig::default_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    #[serde(default = "BrokerConfig::default_backoff_base_ms")]
    pub backoff_base_ms: u64,
    #[serde(default = "BrokerConfig::default_backoff_max_ms")]
    pub backoff_max_ms: u64,
    #[serde(default = "BrokerConfig::default_jitter_frac")]
    pub jitter_frac: f32,
    #[serde(default, deserialize_with = "parse_weights")]
    pub weights: HashMap<String, [u32; 3]>,
    #[serde(default, deserialize_with = "parse_queue_bounds")]
    pub queue_bounds: HashMap<String, usize>,
}

impl BrokerConfig {
    const fn default_max_inflight() -> usize {
        32
    }

    const fn default_per_repo_inflight() -> usize {
        2
    }

    const fn default_cache_capacity() -> usize {
        5000
    }

    const fn default_cache_ttl_secs() -> u64 {
        600
    }

    const fn default_backoff_base_ms() -> u64 {
        500
    }

    const fn default_backoff_max_ms() -> u64 {
        60_000
    }

    const fn default_jitter_frac() -> f32 {
        0.2
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
    #[serde(default = "ObservabilityConfig::default_metrics_bind")]
    pub metrics_bind: String,
}

impl ObservabilityConfig {
    fn default_metrics_path() -> String {
        "/metrics".to_string()
    }

    fn default_metrics_bind() -> String {
        "0.0.0.0:9091".to_string()
    }
}

fn parse_weights<'de, D>(deserializer: D) -> Result<HashMap<String, [u32; 3]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    let mut map = HashMap::new();
    if let Some(raw) = raw {
        for entry in raw.split(';').filter(|s| !s.trim().is_empty()) {
            let mut parts = entry.splitn(2, ':');
            let budget = parts
                .next()
                .ok_or_else(|| serde::de::Error::custom("missing budget in BROKER_WEIGHTS"))?
                .trim()
                .to_string();
            let weights_part = parts
                .next()
                .ok_or_else(|| serde::de::Error::custom("missing weights in BROKER_WEIGHTS"))?;
            let weights: Vec<u32> = weights_part
                .split(',')
                .filter_map(|w| w.trim().parse::<u32>().ok())
                .collect();
            if weights.len() == 3 {
                map.insert(budget, [weights[0], weights[1], weights[2]]);
            }
        }
    }
    Ok(map)
}

fn parse_queue_bounds<'de, D>(deserializer: D) -> Result<HashMap<String, usize>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    let mut map = HashMap::new();
    if let Some(raw) = raw {
        for entry in raw.split(',').filter(|s| !s.trim().is_empty()) {
            let mut parts = entry.splitn(2, ':');
            let key = parts
                .next()
                .ok_or_else(|| serde::de::Error::custom("missing key in BROKER_QUEUE_BOUNDS"))?
                .trim()
                .to_string();
            let val = parts
                .next()
                .and_then(|v| v.trim().parse::<usize>().ok())
                .ok_or_else(|| serde::de::Error::custom("invalid value in BROKER_QUEUE_BOUNDS"))?;
            map.insert(key, val);
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Deserialize)]
    struct ModeWrapper {
        #[serde(default)]
        mode: FetchMode,
    }

    #[test]
    fn fetch_mode_defaults_to_rest() {
        let wrapper: ModeWrapper = serde_json::from_str("{}").unwrap();
        assert_eq!(wrapper.mode, FetchMode::Rest);
    }

    #[test]
    fn fetch_mode_parses_variants() {
        let wrapper: ModeWrapper = serde_json::from_str("{\"mode\":\"graphql\"}").unwrap();
        assert_eq!(wrapper.mode, FetchMode::Graphql);
    }

    #[test]
    fn github_config_parses_csv_tokens() {
        let data = json!({
            "token_ids": "tokenA,tokenB",
            "token_secrets": "secretA,secretB",
            "user_agent": "ua"
        });
        let cfg: GithubConfig = serde_json::from_value(data).expect("config parsed");
        let tokens = cfg.resolved_tokens().expect("tokens resolved");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].id, "tokenA");
        assert_eq!(tokens[1].secret, "secretB");
    }

    #[test]
    fn broker_config_parses_weights_and_bounds() {
        let data = json!({
            "queue_bounds": "core.critical:10,graphql.normal:5",
            "weights": "core:4,2,1;graphql:3,2,1"
        });
        let cfg: BrokerConfig = serde_json::from_value(data).expect("broker config parsed");
        assert_eq!(cfg.queue_bounds.get("core.critical"), Some(&10));
        assert_eq!(cfg.queue_bounds.get("graphql.normal"), Some(&5));
        assert_eq!(cfg.weights.get("core"), Some(&[4, 2, 1]));
    }
}
