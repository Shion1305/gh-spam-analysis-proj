use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use gh_broker::{GithubBroker, Priority};
use http::{header, Request};
use serde_json::Value;
use tracing::instrument;
use url::Url;

#[async_trait]
pub trait GithubClient: Send + Sync {
    async fn get_repo(&self, owner: &str, repo: &str) -> Result<Value>;
    async fn list_repo_issues(
        &self,
        owner: &str,
        repo: &str,
        since: Option<DateTime<Utc>>,
        page: u32,
        per_page: u32,
    ) -> Result<Vec<Value>>;
    async fn list_issue_comments(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        page: u32,
        per_page: u32,
    ) -> Result<Vec<Value>>;
    async fn get_user(&self, login: &str) -> Result<Value>;
}

pub struct BrokerGithubClient {
    broker: Arc<dyn GithubBroker>,
    base: Url,
    user_agent: String,
}

impl BrokerGithubClient {
    pub fn new(broker: Arc<dyn GithubBroker>, user_agent: String) -> Self {
        Self {
            broker,
            base: Url::parse("https://api.github.com/").expect("valid base url"),
            user_agent,
        }
    }

    async fn get_json(&self, url: Url, priority: Priority) -> Result<Value> {
        let response = self.execute(url, priority).await?;
        if response.status().is_success() {
            let body = response.into_body();
            let value: Value = serde_json::from_slice(&body)?;
            Ok(value)
        } else if response.status().as_u16() == 304 {
            Err(anyhow!("received 304 without cached entity"))
        } else {
            Err(anyhow!("github api error: {}", response.status().as_u16()))
        }
    }

    async fn get_json_array(&self, url: Url, priority: Priority) -> Result<Vec<Value>> {
        let value = self.get_json(url, priority).await?;
        match value {
            Value::Array(items) => Ok(items),
            Value::Null => Ok(Vec::new()),
            _ => Err(anyhow!("expected array response")),
        }
    }

    #[instrument(skip(self), fields(url = %url))]
    async fn execute(&self, url: Url, priority: Priority) -> Result<http::Response<Vec<u8>>> {
        let uri: http::Uri = url.as_str().parse()?;
        let request = Request::builder()
            .method("GET")
            .uri(uri)
            .header(header::USER_AGENT, self.user_agent.clone())
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .body(Vec::new())?;

        let response = self.broker.enqueue(request, priority).await?;
        Ok(response)
    }

    fn join(&self, path: &str) -> Result<Url> {
        Ok(self.base.join(path)?)
    }

    fn with_query(url: &mut Url, params: &[(&str, String)]) {
        let mut query_pairs = url.query_pairs_mut();
        for (key, val) in params {
            query_pairs.append_pair(key, val);
        }
    }
}

#[async_trait]
impl GithubClient for BrokerGithubClient {
    async fn get_repo(&self, owner: &str, repo: &str) -> Result<Value> {
        let path = format!("repos/{owner}/{repo}");
        let url = self.join(&path)?;
        self.get_json(url, Priority::Critical).await
    }

    async fn list_repo_issues(
        &self,
        owner: &str,
        repo: &str,
        since: Option<DateTime<Utc>>,
        page: u32,
        per_page: u32,
    ) -> Result<Vec<Value>> {
        let path = format!("repos/{owner}/{repo}/issues");
        let mut url = self.join(&path)?;
        let mut params = vec![
            ("state", "all".to_string()),
            ("sort", "updated".to_string()),
            ("direction", "desc".to_string()),
            ("page", page.to_string()),
            ("per_page", per_page.to_string()),
        ];
        if let Some(since) = since {
            params.push(("since", since.to_rfc3339()));
        }
        Self::with_query(&mut url, &params);
        self.get_json_array(url, Priority::Normal).await
    }

    async fn list_issue_comments(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        page: u32,
        per_page: u32,
    ) -> Result<Vec<Value>> {
        let path = format!("repos/{owner}/{repo}/issues/{issue_number}/comments");
        let mut url = self.join(&path)?;
        let params = [
            ("page", page.to_string()),
            ("per_page", per_page.to_string()),
        ];
        Self::with_query(&mut url, &params);
        self.get_json_array(url, Priority::Normal).await
    }

    async fn get_user(&self, login: &str) -> Result<Value> {
        let path = format!("users/{login}");
        let url = self.join(&path)?;
        self.get_json(url, Priority::Normal).await
    }
}
