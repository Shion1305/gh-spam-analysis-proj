use chrono::{DateTime, Utc};
use http::{header, HeaderMap, HeaderValue, Request};
use serde::Deserialize;
use std::str::FromStr;
use std::time::Duration;
use tracing::debug;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Budget {
    Core,
    Search,
    Graphql,
}

impl Budget {
    pub fn classify(path: &str, resource_header: Option<&HeaderValue>) -> Self {
        if let Some(resource) = resource_header.and_then(|v| v.to_str().ok()) {
            return match resource {
                "search" => Budget::Search,
                "graphql" => Budget::Graphql,
                _ => Budget::Core,
            };
        }

        if path == "/graphql" {
            Budget::Graphql
        } else if path.starts_with("/search/") {
            Budget::Search
        } else {
            Budget::Core
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Priority {
    Critical,
    Normal,
    Backfill,
}

impl Priority {
    pub const ALL: [Priority; 3] = [Priority::Critical, Priority::Normal, Priority::Backfill];

    pub fn as_str(&self) -> &'static str {
        match self {
            Priority::Critical => "critical",
            Priority::Normal => "normal",
            Priority::Backfill => "backfill",
        }
    }
}

#[derive(Debug, Clone)]
pub struct GithubRequest {
    inner: Request<Vec<u8>>,
    pub budget: Budget,
    pub priority: Priority,
    pub key: String,
}

impl GithubRequest {
    pub fn new(mut inner: Request<Vec<u8>>, priority: Priority) -> anyhow::Result<Self> {
        let resource_hdr = inner.headers().get("x-ratelimit-resource").cloned();

        let budget = Budget::classify(inner.uri().path(), resource_hdr.as_ref());

        let key = format!(
            "{} {}{}",
            inner.method(),
            inner.uri().path(),
            inner
                .uri()
                .query()
                .map(|q| format!("?{}", q))
                .unwrap_or_default()
        );

        if !inner.headers().contains_key(header::USER_AGENT) {
            return Err(anyhow::anyhow!("user-agent header required"));
        }

        Ok(Self {
            inner,
            budget,
            priority,
            key,
        })
    }

    pub fn request(&self) -> Request<Vec<u8>> {
        self.inner.clone()
    }

    pub fn headers_mut(&mut self) -> &mut http::HeaderMap {
        self.inner.headers_mut()
    }

    pub fn method(&self) -> &http::Method {
        self.inner.method()
    }

    pub fn uri(&self) -> &http::Uri {
        self.inner.uri()
    }

    pub fn key(&self) -> &str {
        &self.key
    }
}

#[derive(Debug, Clone)]
pub struct RateLimitUpdate {
    pub limit: i64,
    pub remaining: i64,
    pub reset: DateTime<Utc>,
}

pub fn parse_rate_limit(headers: &HeaderMap) -> Option<RateLimitUpdate> {
    let limit = headers
        .get("x-ratelimit-limit")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())?;
    let remaining = headers
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())?;
    let reset_ts = headers
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())?;
    let reset =
        DateTime::<Utc>::from_utc(chrono::NaiveDateTime::from_timestamp_opt(reset_ts, 0)?, Utc);
    Some(RateLimitUpdate {
        limit,
        remaining,
        reset,
    })
}

#[derive(Debug, Clone)]
pub struct RetryAdvice {
    pub wait: Duration,
    pub reason: &'static str,
}

pub fn parse_retry_after(headers: &HeaderMap) -> Option<RetryAdvice> {
    if let Some(value) = headers.get(header::RETRY_AFTER) {
        if let Ok(value) = value.to_str() {
            if let Ok(seconds) = value.parse::<u64>() {
                return Some(RetryAdvice {
                    wait: Duration::from_secs(seconds),
                    reason: "retry_after",
                });
            }
            if let Ok(date) = httpdate::parse_http_date(value) {
                let now = chrono::Utc::now();
                let wait = date - now;
                let dur = wait.to_std().unwrap_or_default();
                return Some(RetryAdvice {
                    wait: dur,
                    reason: "retry_after_date",
                });
            }
        }
    }
    None
}
