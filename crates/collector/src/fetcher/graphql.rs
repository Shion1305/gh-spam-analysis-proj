use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64_ENGINE, Engine as _};
use chrono::{DateTime, Utc};
use gh_broker::{GithubBroker, Priority};
use http::{header, Request, StatusCode};
use normalizer::models::NormalizedUser;
use normalizer::payloads::{CommentPayload, IssuePayload, RepoPayload, UserPayload, UserRef};
use serde_json::{json, Value};
use std::time::Instant;
use tokio::sync::Mutex;

use crate::client::{GithubApiError, GithubClient};
use crate::fetcher::{
    CommentPage, CommentRecord, DataFetcher, IssuePage, IssueRecord, MissingUser, RepoSnapshot,
    UserFetch,
};
use crate::metrics;

const GRAPHQL_ENDPOINT: &str = "https://api.github.com/graphql";

const REPO_QUERY: &str = r#"
query RepoInfo($owner: String!, $name: String!) {
  repository(owner: $owner, name: $name) {
    databaseId
    nameWithOwner
    isFork
    createdAt
    pushedAt
  }
}
"#;

const ISSUES_QUERY: &str = r#"
query RepoIssues(
  $owner: String!,
  $name: String!,
  $perPage: Int!,
  $commentsPerPage: Int!,
  $cursor: String,
  $since: DateTime
) {
  repository(owner: $owner, name: $name) {
    databaseId
    nameWithOwner
    issues(
      first: $perPage,
      after: $cursor,
      orderBy: { field: UPDATED_AT, direction: DESC },
      filterBy: { since: $since }
    ) {
      pageInfo {
        hasNextPage
        endCursor
      }
      nodes {
        databaseId
        number
        title
        body
        state
        createdAt
        updatedAt
        closedAt
        author {
          __typename
          login
          ... on User {
            databaseId
            isSiteAdmin
            createdAt
            followers { totalCount }
            following { totalCount }
            repositories(privacy: PUBLIC) { totalCount }
          }
          ... on Bot {
            databaseId
            createdAt
          }
          ... on Organization {
            databaseId
            createdAt
          }
        }
        comments(
          first: $commentsPerPage,
          orderBy: { field: UPDATED_AT, direction: ASC }
        ) {
          totalCount
          pageInfo {
            hasNextPage
            endCursor
          }
          nodes {
            databaseId
            body
            createdAt
            updatedAt
            author {
              __typename
              login
              ... on User {
                databaseId
                isSiteAdmin
                createdAt
                followers { totalCount }
                following { totalCount }
                repositories(privacy: PUBLIC) { totalCount }
              }
              ... on Bot {
                databaseId
                createdAt
              }
              ... on Organization {
                databaseId
                createdAt
              }
            }
          }
        }
      }
    }
  }
}
"#;

const ISSUE_COMMENTS_QUERY: &str = r#"
query IssueComments(
  $owner: String!,
  $name: String!,
  $number: Int!,
  $perPage: Int!,
  $cursor: String
) {
  repository(owner: $owner, name: $name) {
    issue(number: $number) {
      comments(
        first: $perPage,
        after: $cursor,
        orderBy: { field: UPDATED_AT, direction: ASC }
      ) {
        pageInfo {
          hasNextPage
          endCursor
        }
        nodes {
          databaseId
          body
          createdAt
          updatedAt
          author {
            __typename
            login
            ... on User {
              databaseId
              isSiteAdmin
              createdAt
              followers { totalCount }
              following { totalCount }
              repositories(privacy: PUBLIC) { totalCount }
            }
            ... on Bot {
              databaseId
              createdAt
            }
            ... on Organization {
              databaseId
              createdAt
            }
          }
        }
      }
    }
  }
}
"#;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct IssueKey {
    repo: String,
    number: i64,
}

impl IssueKey {
    fn new(owner: &str, name: &str, number: i64) -> Self {
        Self {
            repo: format!("{}/{}", owner, name),
            number,
        }
    }
}

#[derive(Clone, Debug)]
struct CommentCacheEntry {
    items: Vec<CommentRecord>,
    next_cursor: Option<String>,
}

#[derive(Default)]
struct ActorInfo {
    user_ref: Option<UserRef>,
    normalized_user: Option<NormalizedUser>,
}

pub struct GraphqlDataFetcher {
    broker: Arc<dyn GithubBroker>,
    rest_client: Arc<dyn GithubClient>,
    user_agent: String,
    initial_comments: Mutex<HashMap<IssueKey, CommentCacheEntry>>,
    user_cache: Mutex<HashMap<String, NormalizedUser>>,
}

impl GraphqlDataFetcher {
    pub fn new(
        broker: Arc<dyn GithubBroker>,
        rest_client: Arc<dyn GithubClient>,
        user_agent: String,
    ) -> Self {
        Self {
            broker,
            rest_client,
            user_agent,
            initial_comments: Mutex::new(HashMap::new()),
            user_cache: Mutex::new(HashMap::new()),
        }
    }

    async fn execute_graphql(&self, query: &str, variables: Value) -> Result<Value> {
        let payload = json!({
            "query": query,
            "variables": variables,
        });

        let request = Request::builder()
            .method("POST")
            .uri(GRAPHQL_ENDPOINT)
            .header(header::USER_AGENT, self.user_agent.clone())
            .header(header::ACCEPT, "application/vnd.github+json")
            .header(header::CONTENT_TYPE, "application/json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .body(serde_json::to_vec(&payload)?)?;

        // Capture broker errors and annotate with endpoint for better context
        let response = match self.broker.enqueue(request, Priority::Normal).await {
            Ok(resp) => resp,
            Err(err) => {
                if let Some(http) = err.downcast_ref::<gh_broker::HttpStatusError>() {
                    return Err(GithubApiError::status(http.status, "graphql").into());
                }
                return Err(err);
            }
        };

        let status = response.status();
        if !status.is_success() {
            return Err(GithubApiError::status(status, "graphql").into());
        }

        let body = response.into_body();
        let value: Value = serde_json::from_slice(&body)?;

        if let Some(errors) = value.get("errors").and_then(Value::as_array) {
            return Err(map_graphql_errors(errors));
        }

        Ok(value)
    }

    fn extract_repository<'a>(
        &self,
        root: &'a Value,
        owner: &str,
        name: &str,
    ) -> Result<&'a Value> {
        let repository = root
            .get("data")
            .and_then(|d| d.get("repository"))
            .ok_or_else(|| anyhow!("missing repository field in GraphQL response"))?;
        if repository.is_null() {
            Err(
                GithubApiError::status(StatusCode::NOT_FOUND, format!("repos/{}/{}", owner, name))
                    .into(),
            )
        } else {
            Ok(repository)
        }
    }

    async fn cache_user(&self, user: NormalizedUser) {
        let mut cache = self.user_cache.lock().await;
        cache.insert(user.login.clone(), user);
    }

    async fn store_initial_comments(&self, key: IssueKey, entry: CommentCacheEntry) {
        let mut cache = self.initial_comments.lock().await;
        cache.insert(key, entry);
    }

    async fn take_initial_comments(&self, key: &IssueKey) -> Option<CommentCacheEntry> {
        let mut cache = self.initial_comments.lock().await;
        cache.remove(key)
    }

    fn parse_actor(&self, actor: &Value) -> Result<ActorInfo> {
        if actor.is_null() {
            return Ok(ActorInfo::default());
        }

        let login = match actor.get("login").and_then(Value::as_str) {
            Some(login) if !login.is_empty() => login.to_string(),
            _ => return Ok(ActorInfo::default()),
        };

        let id = match parse_database_id(actor) {
            Some(id) => id,
            None => return Ok(ActorInfo::default()),
        };

        let typename = actor
            .get("__typename")
            .and_then(Value::as_str)
            .unwrap_or("User")
            .to_string();
        let site_admin = actor
            .get("isSiteAdmin")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let created_at = actor
            .get("createdAt")
            .and_then(|v| if v.is_null() { None } else { v.as_str() })
            .map(|value| value.to_string());
        let followers = actor
            .get("followers")
            .and_then(|f| f.get("totalCount"))
            .and_then(Value::as_i64);
        let following = actor
            .get("following")
            .and_then(|f| f.get("totalCount"))
            .and_then(Value::as_i64);
        let public_repos = actor
            .get("repositories")
            .and_then(|r| r.get("totalCount"))
            .and_then(Value::as_i64);

        let user_json = json!({
            "id": id,
            "login": login,
            "type": typename,
            "site_admin": site_admin,
            "created_at": created_at,
            "followers": followers,
            "following": following,
            "public_repos": public_repos,
        });
        let payload: UserPayload = serde_json::from_value(user_json.clone())?;
        let normalized = normalizer::normalize_user(&payload, user_json);

        Ok(ActorInfo {
            user_ref: Some(UserRef { id, login }),
            normalized_user: Some(normalized),
        })
    }

    async fn collect_comment_records(
        &self,
        comments_conn: &Value,
        issue_id: i64,
    ) -> Result<(Vec<CommentRecord>, Option<String>)> {
        let mut records = Vec::new();

        if let Some(nodes) = comments_conn.get("nodes").and_then(Value::as_array) {
            for node in nodes.iter().filter(|node| !node.is_null()) {
                let actor_info = self.parse_actor(node.get("author").unwrap_or(&Value::Null))?;
                if let Some(user) = actor_info.normalized_user.clone() {
                    self.cache_user(user).await;
                }
                let comment_id = node
                    .get("databaseId")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| anyhow!("missing comment databaseId"))?;
                let body = node
                    .get("body")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let created_at = node
                    .get("createdAt")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("missing comment createdAt"))?;
                let updated_at =
                    node.get("updatedAt")
                        .and_then(|v| if v.is_null() { None } else { v.as_str() });

                let user_value = actor_info.user_ref.as_ref().map(user_ref_to_value);
                let comment_value = json!({
                    "id": comment_id,
                    "user": user_value,
                    "body": body,
                    "created_at": created_at,
                    "updated_at": updated_at,
                });
                let payload: CommentPayload = serde_json::from_value(comment_value.clone())?;
                let normalized =
                    normalizer::normalize_comment(&payload, issue_id, comment_value.clone());
                records.push(CommentRecord {
                    comment: normalized,
                    author: actor_info.user_ref.clone(),
                });
            }
        }

        let has_next = comments_conn
            .get("pageInfo")
            .and_then(|p| p.get("hasNextPage"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let next_cursor = if has_next {
            comments_conn
                .get("pageInfo")
                .and_then(|p| p.get("endCursor"))
                .and_then(Value::as_str)
                .map(|s| s.to_string())
        } else {
            None
        };

        Ok((records, next_cursor))
    }
}

#[async_trait]
impl DataFetcher for GraphqlDataFetcher {
    async fn fetch_repo(&self, owner: &str, name: &str) -> Result<RepoSnapshot> {
        let op = "repo";
        let start = Instant::now();
        let response = self
            .execute_graphql(
                REPO_QUERY,
                json!({
                    "owner": owner,
                    "name": name,
                }),
            )
            .await;
        let elapsed = start.elapsed().as_secs_f64();
        let response = match response {
            Ok(v) => {
                metrics::FETCH_REQUESTS_TOTAL
                    .with_label_values(&["graphql", op, "success"])
                    .inc();
                metrics::FETCH_LATENCY_SECONDS
                    .with_label_values(&["graphql", op])
                    .observe(elapsed);
                v
            }
            Err(e) => {
                metrics::FETCH_REQUESTS_TOTAL
                    .with_label_values(&["graphql", op, "error"])
                    .inc();
                metrics::FETCH_LATENCY_SECONDS
                    .with_label_values(&["graphql", op])
                    .observe(elapsed);
                return Err(e);
            }
        };

        let repository = self.extract_repository(&response, owner, name)?;

        let id = repository
            .get("databaseId")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow!("missing repository databaseId"))?;
        let full_name = repository
            .get("nameWithOwner")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing repository nameWithOwner"))?;
        let is_fork = repository
            .get("isFork")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let created_at = repository
            .get("createdAt")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing repository createdAt"))?;
        let pushed_at =
            repository
                .get("pushedAt")
                .and_then(|v| if v.is_null() { None } else { v.as_str() });

        let repo_value = json!({
            "id": id,
            "full_name": full_name,
            "fork": is_fork,
            "created_at": created_at,
            "pushed_at": pushed_at,
        });
        let payload: RepoPayload = serde_json::from_value(repo_value.clone())?;
        let repository = normalizer::normalize_repo(&payload, repo_value);
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
        let op = "issues";
        let per_page = per_page.min(100);
        let comments_per_page = per_page;
        let since_value = since.map(|dt| dt.to_rfc3339());
        let start = Instant::now();
        let response = self
            .execute_graphql(
                ISSUES_QUERY,
                json!({
                    "owner": owner,
                    "name": name,
                    "perPage": per_page as i64,
                    "commentsPerPage": comments_per_page as i64,
                    "cursor": cursor,
                    "since": since_value,
                }),
            )
            .await;
        let elapsed = start.elapsed().as_secs_f64();
        let response = match response {
            Ok(v) => {
                metrics::FETCH_REQUESTS_TOTAL
                    .with_label_values(&["graphql", op, "success"])
                    .inc();
                metrics::FETCH_LATENCY_SECONDS
                    .with_label_values(&["graphql", op])
                    .observe(elapsed);
                v
            }
            Err(e) => {
                metrics::FETCH_REQUESTS_TOTAL
                    .with_label_values(&["graphql", op, "error"])
                    .inc();
                metrics::FETCH_LATENCY_SECONDS
                    .with_label_values(&["graphql", op])
                    .observe(elapsed);
                return Err(e);
            }
        };

        let repository = self.extract_repository(&response, owner, name)?;

        let issues_conn = repository
            .get("issues")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("missing issues connection"))?;

        let has_next = issues_conn
            .get("pageInfo")
            .and_then(|p| p.get("hasNextPage"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let next_cursor = if has_next {
            issues_conn
                .get("pageInfo")
                .and_then(|p| p.get("endCursor"))
                .and_then(Value::as_str)
                .map(|s| s.to_string())
        } else {
            None
        };

        let mut items = Vec::new();
        if let Some(nodes) = issues_conn.get("nodes").and_then(Value::as_array) {
            for node in nodes.iter().filter(|node| !node.is_null()) {
                let actor_info = self.parse_actor(node.get("author").unwrap_or(&Value::Null))?;
                if let Some(user) = actor_info.normalized_user.clone() {
                    self.cache_user(user).await;
                }

                let issue_id = node
                    .get("databaseId")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| anyhow!("missing issue databaseId"))?;
                let issue_number = node
                    .get("number")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| anyhow!("missing issue number"))?;
                let title = node
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let body = node.get("body").and_then(Value::as_str);
                let state = node
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or("OPEN")
                    .to_string();
                let created_at = node
                    .get("createdAt")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("missing issue createdAt"))?;
                let updated_at = node
                    .get("updatedAt")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("missing issue updatedAt"))?;
                let closed_at =
                    node.get("closedAt")
                        .and_then(|v| if v.is_null() { None } else { v.as_str() });

                let comments_conn = node
                    .get("comments")
                    .ok_or_else(|| anyhow!("missing comments connection"))?;
                let comments_total = comments_conn
                    .get("totalCount")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);

                // The `issues` connection only returns issues (not pull requests).
                // For REST compatibility, the normalizer expects `pull_request: null` for issues.
                let pull_request_value = None::<Value>;
                let user_value = actor_info.user_ref.as_ref().map(user_ref_to_value);
                let issue_value = json!({
                    "id": issue_id,
                    "number": issue_number,
                    "pull_request": pull_request_value,
                    "state": state,
                    "title": title,
                    "body": body,
                    "user": user_value,
                    "comments": comments_total,
                    "created_at": created_at,
                    "updated_at": updated_at,
                    "closed_at": closed_at,
                });
                let payload: IssuePayload = serde_json::from_value(issue_value.clone())?;
                let normalized =
                    normalizer::normalize_issue(&payload, repo_id, issue_value.clone());

                let (comment_records, comment_cursor) = self
                    .collect_comment_records(comments_conn, normalized.id)
                    .await?;
                if !comment_records.is_empty() || comment_cursor.is_some() {
                    let key = IssueKey::new(owner, name, issue_number);
                    self.store_initial_comments(
                        key,
                        CommentCacheEntry {
                            items: comment_records,
                            next_cursor: comment_cursor,
                        },
                    )
                    .await;
                }

                items.push(IssueRecord {
                    issue: normalized,
                    author: payload.user.clone(),
                });
            }
        }

        metrics::FETCH_ITEMS_TOTAL
            .with_label_values(&["graphql", op])
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
        let key = IssueKey::new(owner, name, issue_number);
        if cursor.is_none() {
            if let Some(entry) = self.take_initial_comments(&key).await {
                return Ok(CommentPage {
                    items: entry.items,
                    next_cursor: entry.next_cursor,
                });
            }
        }

        let per_page = per_page.min(100);
        let start = Instant::now();
        let response = self
            .execute_graphql(
                ISSUE_COMMENTS_QUERY,
                json!({
                    "owner": owner,
                    "name": name,
                    "number": issue_number,
                    "perPage": per_page as i64,
                    "cursor": cursor,
                }),
            )
            .await;
        let elapsed = start.elapsed().as_secs_f64();
        let response = match response {
            Ok(v) => {
                metrics::FETCH_REQUESTS_TOTAL
                    .with_label_values(&["graphql", op, "success"])
                    .inc();
                metrics::FETCH_LATENCY_SECONDS
                    .with_label_values(&["graphql", op])
                    .observe(elapsed);
                v
            }
            Err(e) => {
                metrics::FETCH_REQUESTS_TOTAL
                    .with_label_values(&["graphql", op, "error"])
                    .inc();
                metrics::FETCH_LATENCY_SECONDS
                    .with_label_values(&["graphql", op])
                    .observe(elapsed);
                return Err(e);
            }
        };

        let repository = self.extract_repository(&response, owner, name)?;
        let issue_value = repository
            .get("issue")
            .ok_or_else(|| anyhow!("missing issue field in GraphQL response"))?;
        if issue_value.is_null() {
            // Issue not found; treat as benign skip (empty comments page)
            metrics::FETCH_REQUESTS_TOTAL
                .with_label_values(&["graphql", op, "success"])
                .inc();
            metrics::FETCH_LATENCY_SECONDS
                .with_label_values(&["graphql", op])
                .observe(elapsed);
            crate::metrics::ISSUES_404_SKIPS_TOTAL.inc();
            return Ok(CommentPage {
                items: Vec::new(),
                next_cursor: None,
            });
        }

        let comments_conn = issue_value
            .get("comments")
            .ok_or_else(|| anyhow!("missing comments connection"))?;

        let (items, next_cursor) = self
            .collect_comment_records(comments_conn, issue_id)
            .await?;

        metrics::FETCH_ITEMS_TOTAL
            .with_label_values(&["graphql", op])
            .inc_by(items.len() as u64);
        Ok(CommentPage { items, next_cursor })
    }

    async fn fetch_user(&self, user: &UserRef) -> Result<UserFetch> {
        let op = "user";
        let start = Instant::now();
        {
            let cache = self.user_cache.lock().await;
            if let Some(user_data) = cache.get(&user.login) {
                metrics::FETCH_REQUESTS_TOTAL
                    .with_label_values(&["graphql", op, "success"])
                    .inc();
                metrics::FETCH_LATENCY_SECONDS
                    .with_label_values(&["graphql", op])
                    .observe(start.elapsed().as_secs_f64());
                return Ok(UserFetch::Found(user_data.clone()));
            }
        }

        let result = self.rest_client.get_user(&user.login).await;
        let elapsed = start.elapsed().as_secs_f64();
        match result {
            Ok(value) => {
                let payload: UserPayload = serde_json::from_value(value.clone())?;
                let normalized = normalizer::normalize_user(&payload, value);
                self.cache_user(normalized.clone()).await;
                metrics::FETCH_REQUESTS_TOTAL
                    .with_label_values(&["graphql", op, "success"])
                    .inc();
                metrics::FETCH_LATENCY_SECONDS
                    .with_label_values(&["graphql", op])
                    .observe(elapsed);
                Ok(UserFetch::Found(normalized))
            }
            Err(err) => {
                if let Some(api_err) = err.downcast_ref::<GithubApiError>() {
                    if matches!(api_err.status_code(), StatusCode::NOT_FOUND) {
                        metrics::FETCH_REQUESTS_TOTAL
                            .with_label_values(&["graphql", op, "success"])
                            .inc();
                        metrics::FETCH_LATENCY_SECONDS
                            .with_label_values(&["graphql", op])
                            .observe(elapsed);
                        crate::metrics::USERS_404_SKIPS_TOTAL.inc();
                        return Ok(UserFetch::Missing(MissingUser {
                            id: user.id,
                            login: user.login.clone(),
                            status: Some(api_err.status_code()),
                        }));
                    }
                } else if let Some(http_status) = err.downcast_ref::<gh_broker::HttpStatusError>() {
                    if http_status.status == StatusCode::NOT_FOUND {
                        metrics::FETCH_REQUESTS_TOTAL
                            .with_label_values(&["graphql", op, "success"])
                            .inc();
                        metrics::FETCH_LATENCY_SECONDS
                            .with_label_values(&["graphql", op])
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
                    .with_label_values(&["graphql", op, "error"])
                    .inc();
                metrics::FETCH_LATENCY_SECONDS
                    .with_label_values(&["graphql", op])
                    .observe(elapsed);
                Err(err)
            }
        }
    }
}

fn user_ref_to_value(user: &UserRef) -> Value {
    json!({
        "id": user.id,
        "login": user.login,
    })
}

fn parse_database_id(actor: &Value) -> Option<i64> {
    if let Some(id) = actor.get("databaseId").and_then(Value::as_i64) {
        return Some(id);
    }
    let global_id = actor.get("id")?.as_str()?;
    decode_global_id(global_id)
}

fn decode_global_id(value: &str) -> Option<i64> {
    let decoded = BASE64_ENGINE.decode(value).ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let trailing_digits: String = text
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    if trailing_digits.is_empty() {
        None
    } else {
        trailing_digits
            .chars()
            .rev()
            .collect::<String>()
            .parse()
            .ok()
    }
}

fn map_graphql_errors(errors: &[Value]) -> anyhow::Error {
    if let Some(first) = errors.first() {
        let message = first
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown GraphQL error");
        let error_type = first
            .get("type")
            .or_else(|| first.get("extensions").and_then(|ext| ext.get("code")))
            .and_then(Value::as_str)
            .unwrap_or("");
        if error_type == "NOT_FOUND" {
            return GithubApiError::status(StatusCode::NOT_FOUND, "graphql").into();
        }
        return anyhow!(message.to_string());
    }
    anyhow!("unknown GraphQL error")
}
