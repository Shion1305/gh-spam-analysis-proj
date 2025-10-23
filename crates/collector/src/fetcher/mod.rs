use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use http::StatusCode;
use normalizer::models::{
    NormalizedComment, NormalizedIssue, NormalizedRepository, NormalizedUser,
};
use normalizer::payloads::UserRef;

pub mod graphql;
pub mod rest;

pub use graphql::GraphqlDataFetcher;
pub use rest::RestDataFetcher;

#[async_trait]
pub trait DataFetcher: Send + Sync {
    async fn fetch_repo(&self, owner: &str, name: &str) -> Result<RepoSnapshot>;

    async fn fetch_issues(
        &self,
        owner: &str,
        name: &str,
        repo_id: i64,
        since: Option<DateTime<Utc>>,
        cursor: Option<String>,
        per_page: u32,
    ) -> Result<IssuePage>;

    async fn fetch_issue_comments(
        &self,
        owner: &str,
        name: &str,
        issue_number: i64,
        issue_id: i64,
        cursor: Option<String>,
        per_page: u32,
    ) -> Result<CommentPage>;

    async fn fetch_user(&self, user: &UserRef) -> Result<UserFetch>;
}

#[derive(Debug, Clone)]
pub struct RepoSnapshot {
    pub repository: NormalizedRepository,
}

#[derive(Debug, Clone)]
pub struct IssueRecord {
    pub issue: NormalizedIssue,
    pub author: Option<UserRef>,
}

#[derive(Debug, Clone)]
pub struct IssuePage {
    pub items: Vec<IssueRecord>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CommentRecord {
    pub comment: NormalizedComment,
    pub author: Option<UserRef>,
}

#[derive(Debug, Clone)]
pub struct CommentPage {
    pub items: Vec<CommentRecord>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MissingUser {
    pub id: i64,
    pub login: String,
    pub status: Option<StatusCode>,
}

#[derive(Debug, Clone)]
pub enum UserFetch {
    Found(NormalizedUser),
    Missing(MissingUser),
}

pub type SharedFetcher = Arc<dyn DataFetcher>;
