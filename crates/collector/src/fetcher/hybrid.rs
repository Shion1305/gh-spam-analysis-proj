use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::client::GithubClient;
use crate::fetcher::{
    CommentPage, DataFetcher, GraphqlDataFetcher, IssuePage, RepoSnapshot, RestDataFetcher,
    UserFetch,
};
use normalizer::payloads::UserRef;

/// Hybrid fetcher:
/// - Repo metadata via GraphQL
/// - Issues listing via REST (ensures PRs included)
/// - Comments via REST (works for both issues and PRs)
/// - Users via REST (GraphQL user caching is less critical here)
pub struct HybridDataFetcher {
    graphql: GraphqlDataFetcher,
    rest: RestDataFetcher,
}

impl HybridDataFetcher {
    pub fn new(
        broker: Arc<dyn gh_broker::GithubBroker>,
        rest_client: Arc<dyn GithubClient>,
        user_agent: String,
    ) -> Self {
        let graphql = GraphqlDataFetcher::new(broker, rest_client.clone(), user_agent);
        let rest = RestDataFetcher::new(rest_client);
        Self { graphql, rest }
    }
}

#[async_trait]
impl DataFetcher for HybridDataFetcher {
    async fn fetch_repo(&self, owner: &str, name: &str) -> Result<RepoSnapshot> {
        self.graphql.fetch_repo(owner, name).await
    }

    async fn fetch_issues(
        &self,
        owner: &str,
        name: &str,
        repo_id: i64,
        since: Option<DateTime<Utc>>,
        cursor: Option<String>,
        per_page: u32,
    ) -> Result<IssuePage> {
        self.rest
            .fetch_issues(owner, name, repo_id, since, cursor, per_page)
            .await
    }

    async fn fetch_issue_comments(
        &self,
        owner: &str,
        name: &str,
        issue_number: i64,
        issue_id: i64,
        cursor: Option<String>,
        per_page: u32,
    ) -> Result<CommentPage> {
        self.rest
            .fetch_issue_comments(owner, name, issue_number, issue_id, cursor, per_page)
            .await
    }

    async fn fetch_user(&self, user: &UserRef) -> Result<UserFetch> {
        self.rest.fetch_user(user).await
    }
}
