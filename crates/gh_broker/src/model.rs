use chrono::{DateTime, Utc};
use http::{header, HeaderMap, HeaderValue, Request};
use sha2::{Digest, Sha256};
use std::time::Duration;

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

#[derive(Debug)]
pub struct GithubRequest {
    inner: Request<Vec<u8>>,
    pub budget: Budget,
    pub priority: Priority,
    pub key: String,
}

impl Clone for GithubRequest {
    fn clone(&self) -> Self {
        let mut builder = Request::builder()
            .method(self.inner.method().clone())
            .uri(self.inner.uri().clone())
            .version(self.inner.version());

        for (key, value) in self.inner.headers().iter() {
            builder = builder.header(key, value);
        }

        let inner = builder.body(self.inner.body().clone()).unwrap();

        Self {
            inner,
            budget: self.budget,
            priority: self.priority,
            key: self.key.clone(),
        }
    }
}

impl GithubRequest {
    pub fn new(inner: Request<Vec<u8>>, priority: Priority) -> anyhow::Result<Self> {
        let resource_hdr = inner.headers().get("x-ratelimit-resource").cloned();

        let budget = Budget::classify(inner.uri().path(), resource_hdr.as_ref());

        // Base key: method + path + query
        let mut key = format!(
            "{} {}{}",
            inner.method(),
            inner.uri().path(),
            inner
                .uri()
                .query()
                .map(|q| format!("?{}", q))
                .unwrap_or_default()
        );

        // For non-GET requests (e.g., GraphQL POST /graphql), include a short body hash
        // to avoid coalescing unrelated requests with identical method/path/query.
        if inner.method() != http::Method::GET {
            let mut hasher = Sha256::new();
            hasher.update(inner.body());
            let digest = hasher.finalize();
            let hex = format!("{:x}", digest);
            // Use first 16 hex chars to keep the key compact
            let short = &hex[..16.min(hex.len())];
            key.push_str(" body:");
            key.push_str(short);
        }

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
        self.clone().inner
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
    let reset = DateTime::from_timestamp(reset_ts, 0)?;
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
                let now = std::time::SystemTime::now();
                if let Ok(wait) = date.duration_since(now) {
                    return Some(RetryAdvice {
                        wait,
                        reason: "retry_after_date",
                    });
                }
            }
        }
    }
    None
}
