use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use collector::client::{GithubApiError, GithubClient};
use collector::fetcher::{DataFetcher, GraphqlDataFetcher, RepoSnapshot, UserFetch};
use gh_broker::{GithubBroker, Priority};
use http::{Request, Response, StatusCode};
use normalizer::payloads::UserRef;
use serde_json::json;

struct StubBroker;

impl GithubBroker for StubBroker {
    fn enqueue(
        &self,
        _request: Request<Vec<u8>>,
        _priority: Priority,
    ) -> futures::future::BoxFuture<'static, Result<Response<Vec<u8>>>> {
        // Return a minimal GraphQL response for REPO query
        let body = json!({
            "data": {
                "repository": {
                    "databaseId": 9999,
                    "nameWithOwner": "owner/example",
                    "isFork": false,
                    "createdAt": "2021-01-01T00:00:00Z",
                    "pushedAt": "2021-01-02T00:00:00Z"
                }
            }
        })
        .to_string()
        .into_bytes();
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(body)
            .unwrap();
        Box::pin(async move { Ok(resp) })
    }
}

struct StubClientFound;

#[async_trait]
impl GithubClient for StubClientFound {
    async fn get_repo(&self, _owner: &str, _repo: &str) -> Result<serde_json::Value> {
        unreachable!()
    }
    async fn list_repo_issues(
        &self,
        _owner: &str,
        _repo: &str,
        _since: Option<chrono::DateTime<Utc>>,
        _page: u32,
        _per_page: u32,
    ) -> Result<Vec<serde_json::Value>> {
        unreachable!()
    }
    async fn list_issue_comments(
        &self,
        _owner: &str,
        _repo: &str,
        _issue_number: u64,
        _page: u32,
        _per_page: u32,
    ) -> Result<Vec<serde_json::Value>> {
        unreachable!()
    }
    async fn get_user(&self, login: &str) -> Result<serde_json::Value> {
        let value = json!({
            "id": 1234,
            "login": login,
            "type": "User",
            "site_admin": false,
            "created_at": "2020-01-01T00:00:00Z",
            "followers": 10,
            "following": 2,
            "public_repos": 5
        });
        Ok(value)
    }
}

struct StubClient404;

#[async_trait]
impl GithubClient for StubClient404 {
    async fn get_repo(&self, _owner: &str, _repo: &str) -> Result<serde_json::Value> {
        unreachable!()
    }
    async fn list_repo_issues(
        &self,
        _owner: &str,
        _repo: &str,
        _since: Option<chrono::DateTime<Utc>>,
        _page: u32,
        _per_page: u32,
    ) -> Result<Vec<serde_json::Value>> {
        unreachable!()
    }
    async fn list_issue_comments(
        &self,
        _owner: &str,
        _repo: &str,
        _issue_number: u64,
        _page: u32,
        _per_page: u32,
    ) -> Result<Vec<serde_json::Value>> {
        unreachable!()
    }
    async fn get_user(&self, login: &str) -> Result<serde_json::Value> {
        Err(GithubApiError::status(StatusCode::NOT_FOUND, format!("users/{login}")).into())
    }
}

#[tokio::test]
async fn hybrid_fetcher_repo_uses_graphql() -> Result<()> {
    let fetcher =
        GraphqlDataFetcher::new(Arc::new(StubBroker), Arc::new(StubClientFound), "ua".into());
    let snapshot: RepoSnapshot = fetcher.fetch_repo("owner", "example").await?;
    assert_eq!(snapshot.repository.full_name, "owner/example");
    assert_eq!(snapshot.repository.id, 9999);
    Ok(())
}

#[tokio::test]
async fn hybrid_fetcher_user_found_via_rest() -> Result<()> {
    let fetcher =
        GraphqlDataFetcher::new(Arc::new(StubBroker), Arc::new(StubClientFound), "ua".into());
    let user_ref = UserRef {
        id: 1234,
        login: "alice".to_string(),
    };
    match fetcher.fetch_user(&user_ref).await? {
        UserFetch::Found(user) => {
            assert_eq!(user.login, "alice");
            Ok(())
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

#[tokio::test]
async fn hybrid_fetcher_user_missing_on_404() -> Result<()> {
    let fetcher =
        GraphqlDataFetcher::new(Arc::new(StubBroker), Arc::new(StubClient404), "ua".into());
    let user_ref = UserRef {
        id: 5678,
        login: "ghost".to_string(),
    };
    match fetcher.fetch_user(&user_ref).await? {
        UserFetch::Missing(m) => {
            assert_eq!(m.login, "ghost");
            assert_eq!(m.id, 5678);
            Ok(())
        }
        other => panic!("expected Missing, got {other:?}"),
    }
}
