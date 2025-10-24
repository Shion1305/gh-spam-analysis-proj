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
use std::time::Instant;

use crate::client::{GithubApiError, GithubClient};
use crate::fetcher::{
    CommentPage, CommentRecord, DataFetcher, IssuePage, IssueRecord, MissingUser, RepoSnapshot,
    UserFetch,
};
use crate::metrics;

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
        let op = "repo";
        let start = Instant::now();
        let result = async {
            let repo_value = self.client.get_repo(owner, name).await?;
            let repo_payload: RepoPayload = serde_json::from_value(repo_value.clone())?;
            let repository = normalize_repo(&repo_payload, repo_value);
            Ok::<_, anyhow::Error>(RepoSnapshot { repository })
        }
        .await;
        let elapsed = start.elapsed().as_secs_f64();
        match &result {
            Ok(_) => {
                metrics::FETCH_REQUESTS_TOTAL
                    .with_label_values(&["rest", op, "success"])
                    .inc();
                metrics::FETCH_LATENCY_SECONDS
                    .with_label_values(&["rest", op])
                    .observe(elapsed);
            }
            Err(_) => {
                metrics::FETCH_REQUESTS_TOTAL
                    .with_label_values(&["rest", op, "error"])
                    .inc();
                metrics::FETCH_LATENCY_SECONDS
                    .with_label_values(&["rest", op])
                    .observe(elapsed);
            }
        }
        result
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
        let op = "issues";
        let start = Instant::now();
        let page = cursor
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|page| *page > 0)
            .unwrap_or(1);

        let issues_result = self
            .client
            .list_repo_issues(owner, name, since, page, per_page)
            .await;
        let elapsed = start.elapsed().as_secs_f64();
        let issues = match issues_result {
            Ok(v) => {
                metrics::FETCH_REQUESTS_TOTAL
                    .with_label_values(&["rest", op, "success"])
                    .inc();
                metrics::FETCH_LATENCY_SECONDS
                    .with_label_values(&["rest", op])
                    .observe(elapsed);
                v
            }
            Err(e) => {
                metrics::FETCH_REQUESTS_TOTAL
                    .with_label_values(&["rest", op, "error"])
                    .inc();
                metrics::FETCH_LATENCY_SECONDS
                    .with_label_values(&["rest", op])
                    .observe(elapsed);
                return Err(e);
            }
        };

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

        metrics::FETCH_ITEMS_TOTAL
            .with_label_values(&["rest", op])
            .inc_by(items.len() as u64);
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
        let op = "comments";
        let start = Instant::now();
        let page = cursor
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|page| *page > 0)
            .unwrap_or(1);

        let comments_result = self
            .client
            .list_issue_comments(owner, name, issue_number as u64, page, per_page)
            .await;
        let elapsed = start.elapsed().as_secs_f64();
        let comments = match comments_result {
            Ok(v) => {
                metrics::FETCH_REQUESTS_TOTAL
                    .with_label_values(&["rest", op, "success"])
                    .inc();
                metrics::FETCH_LATENCY_SECONDS
                    .with_label_values(&["rest", op])
                    .observe(elapsed);
                v
            }
            Err(e) => {
                // Treat 404 on comments as empty page to avoid failing the job
                let not_found = if let Some(api_err) = e.downcast_ref::<GithubApiError>() {
                    api_err.status_code() == StatusCode::NOT_FOUND
                } else if let Some(status_err) = e.downcast_ref::<gh_broker::HttpStatusError>() {
                    status_err.status == StatusCode::NOT_FOUND
                } else {
                    false
                };

                if not_found {
                    metrics::FETCH_REQUESTS_TOTAL
                        .with_label_values(&["rest", op, "success"])
                        .inc();
                    metrics::FETCH_LATENCY_SECONDS
                        .with_label_values(&["rest", op])
                        .observe(elapsed);
                    crate::metrics::COMMENTS_404_SKIPS_TOTAL.inc();
                    Vec::new()
                } else {
                    metrics::FETCH_REQUESTS_TOTAL
                        .with_label_values(&["rest", op, "error"])
                        .inc();
                    metrics::FETCH_LATENCY_SECONDS
                        .with_label_values(&["rest", op])
                        .observe(elapsed);
                    return Err(e);
                }
            }
        };

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

        metrics::FETCH_ITEMS_TOTAL
            .with_label_values(&["rest", op])
            .inc_by(items.len() as u64);
        Ok(CommentPage { items, next_cursor })
    }

    async fn fetch_user(&self, user: &UserRef) -> Result<UserFetch> {
        let op = "user";
        let start = Instant::now();
        let result = self.client.get_user(&user.login).await;
        let elapsed = start.elapsed().as_secs_f64();
        match result {
            Ok(user_value) => {
                let payload: UserPayload = serde_json::from_value(user_value.clone())?;
                let normalized = normalize_user(&payload, user_value);
                metrics::FETCH_REQUESTS_TOTAL
                    .with_label_values(&["rest", op, "success"])
                    .inc();
                metrics::FETCH_LATENCY_SECONDS
                    .with_label_values(&["rest", op])
                    .observe(elapsed);
                Ok(UserFetch::Found(normalized))
            }
            Err(err) => {
                if let Some(api_err) = err.downcast_ref::<GithubApiError>() {
                    if matches!(api_err.status_code(), StatusCode::NOT_FOUND) {
                        metrics::FETCH_REQUESTS_TOTAL
                            .with_label_values(&["rest", op, "success"])
                            .inc();
                        metrics::FETCH_LATENCY_SECONDS
                            .with_label_values(&["rest", op])
                            .observe(elapsed);
                        crate::metrics::USERS_404_SKIPS_TOTAL.inc();
                        return Ok(UserFetch::Missing(MissingUser {
                            id: user.id,
                            login: user.login.clone(),
                            status: Some(api_err.status_code()),
                        }));
                    }
                } else if let Some(http_err) = err.downcast_ref::<crate::client::GithubApiError>() {
                    // Redundant, but keep branch symmetry
                    if matches!(http_err.status_code(), StatusCode::NOT_FOUND) {
                        metrics::FETCH_REQUESTS_TOTAL
                            .with_label_values(&["rest", op, "success"])
                            .inc();
                        metrics::FETCH_LATENCY_SECONDS
                            .with_label_values(&["rest", op])
                            .observe(elapsed);
                        crate::metrics::USERS_404_SKIPS_TOTAL.inc();
                        return Ok(UserFetch::Missing(MissingUser {
                            id: user.id,
                            login: user.login.clone(),
                            status: Some(http_err.status_code()),
                        }));
                    }
                } else if let Some(http_status) = err.downcast_ref::<gh_broker::HttpStatusError>() {
                    if http_status.status == StatusCode::NOT_FOUND {
                        metrics::FETCH_REQUESTS_TOTAL
                            .with_label_values(&["rest", op, "success"])
                            .inc();
                        metrics::FETCH_LATENCY_SECONDS
                            .with_label_values(&["rest", op])
                            .observe(elapsed);
                        crate::metrics::USERS_404_SKIPS_TOTAL.inc();
                        return Ok(UserFetch::Missing(MissingUser {
                            id: user.id,
                            login: user.login.clone(),
                            status: Some(http_status.status),
                        }));
                    }
                }
                metrics::FETCH_REQUESTS_TOTAL
                    .with_label_values(&["rest", op, "error"])
                    .inc();
                metrics::FETCH_LATENCY_SECONDS
                    .with_label_values(&["rest", op])
                    .observe(elapsed);
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
