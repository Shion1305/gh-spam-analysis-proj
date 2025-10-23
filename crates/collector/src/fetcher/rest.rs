use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use http::StatusCode;
use normalizer::models::{
    NormalizedComment, NormalizedIssue, NormalizedRepository, NormalizedUser,
};
use normalizer::payloads::{CommentPayload, IssuePayload, RepoPayload, UserPayload, UserRef};
use serde_json::Value;

use crate::client::{GithubApiError, GithubClient};
use crate::fetcher::{
    CommentPage, CommentRecord, DataFetcher, IssuePage, IssueRecord, MissingUser, RepoSnapshot,
    UserFetch,
};

pub struct RestDataFetcher {
    client: Arc<dyn GithubClient>,
}

impl RestDataFetcher {
    pub fn new(client: Arc<dyn GithubClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl DataFetcher for RestDataFetcher {
    async fn fetch_repo(&self, owner: &str, name: &str) -> Result<RepoSnapshot> {
        let repo_value = self.client.get_repo(owner, name).await?;
        let repo_payload: RepoPayload = serde_json::from_value(repo_value.clone())?;
        let repository = normalize_repo(&repo_payload, repo_value);
        Ok(RepoSnapshot { repository })
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
        let page = cursor
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|page| *page > 0)
            .unwrap_or(1);

        let issues = self
            .client
            .list_repo_issues(owner, name, since, page, per_page)
            .await?;

        let mut items = Vec::with_capacity(issues.len());

        for issue_value in issues {
            let issue_payload: IssuePayload = serde_json::from_value(issue_value.clone())?;
            let normalized = normalize_issue(&issue_payload, repo_id, issue_value);
            items.push(IssueRecord {
                issue: normalized,
                author: issue_payload.user,
            });
        }

        let next_cursor = if items.len() == per_page as usize {
            Some((page + 1).to_string())
        } else {
            None
        };

        Ok(IssuePage { items, next_cursor })
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
        let page = cursor
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|page| *page > 0)
            .unwrap_or(1);

        let comments = self
            .client
            .list_issue_comments(owner, name, issue_number as u64, page, per_page)
            .await?;

        let mut items = Vec::with_capacity(comments.len());

        for comment_value in comments {
            let comment_payload: CommentPayload = serde_json::from_value(comment_value.clone())?;
            let normalized = normalize_comment(&comment_payload, issue_id, comment_value);
            items.push(CommentRecord {
                comment: normalized,
                author: comment_payload.user,
            });
        }

        let next_cursor = if items.len() == per_page as usize {
            Some((page + 1).to_string())
        } else {
            None
        };

        Ok(CommentPage { items, next_cursor })
    }

    async fn fetch_user(&self, user: &UserRef) -> Result<UserFetch> {
        match self.client.get_user(&user.login).await {
            Ok(user_value) => {
                let payload: UserPayload = serde_json::from_value(user_value.clone())?;
                let normalized = normalize_user(&payload, user_value);
                Ok(UserFetch::Found(normalized))
            }
            Err(err) => {
                if let Some(api_err) = err.downcast_ref::<GithubApiError>() {
                    if matches!(api_err.status_code(), StatusCode::NOT_FOUND) {
                        return Ok(UserFetch::Missing(MissingUser {
                            id: user.id,
                            login: user.login.clone(),
                            status: Some(api_err.status_code()),
                        }));
                    }
                }
                Err(err)
            }
        }
    }
}

fn normalize_repo(payload: &RepoPayload, raw: Value) -> NormalizedRepository {
    normalizer::normalize_repo(payload, raw)
}

fn normalize_issue(payload: &IssuePayload, repo_id: i64, raw: Value) -> NormalizedIssue {
    normalizer::normalize_issue(payload, repo_id, raw)
}

fn normalize_comment(payload: &CommentPayload, issue_id: i64, raw: Value) -> NormalizedComment {
    normalizer::normalize_comment(payload, issue_id, raw)
}

fn normalize_user(payload: &UserPayload, raw: Value) -> NormalizedUser {
    normalizer::normalize_user(payload, raw)
}
