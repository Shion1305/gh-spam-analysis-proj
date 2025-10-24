use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::Utc;
use collector::fetcher::{
    CommentPage, DataFetcher, IssuePage, IssueRecord, RepoSnapshot, UserFetch,
};
use collector::service::Collector;
use common::config::{CollectorConfig, FetchMode};
use db::models::{CollectionJobCreate, IssueQuery};
use db::pg::PgDatabase;
use db::Repositories;
use db_test_fixture::DbFixture;
use http::StatusCode;
use normalizer::models::{NormalizedIssue, NormalizedRepository};
use normalizer::payloads::UserRef;
use serde_json::json;

fn cfg() -> CollectorConfig {
    CollectorConfig {
        page_size: 50,
        interval_secs: 0,
        run_once: true,
        fetch_mode: FetchMode::Hybrid,
        max_concurrent_repos: 1,
    }
}

struct MissingUserFetcher;

#[async_trait]
impl DataFetcher for MissingUserFetcher {
    async fn fetch_repo(&self, owner: &str, name: &str) -> Result<RepoSnapshot> {
        Ok(RepoSnapshot {
            repository: NormalizedRepository {
                id: 9001,
                full_name: format!("{}/{}", owner, name),
                is_fork: false,
                created_at: Utc::now(),
                pushed_at: None,
                raw: json!({}),
            },
        })
    }

    async fn fetch_issues(
        &self,
        _owner: &str,
        _name: &str,
        repo_id: i64,
        _since: Option<chrono::DateTime<Utc>>,
        _cursor: Option<String>,
        _per_page: u32,
    ) -> Result<IssuePage> {
        let issue = NormalizedIssue {
            id: 9002,
            repo_id,
            number: 1,
            is_pull_request: false,
            state: "open".into(),
            title: "t".into(),
            body: None,
            user_id: Some(4242),
            comments_count: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            closed_at: None,
            dedupe_hash: "h".into(),
            raw: json!({}),
        };
        Ok(IssuePage {
            items: vec![IssueRecord {
                issue,
                author: Some(UserRef {
                    id: 4242,
                    login: "ghost-404".into(),
                }),
            }],
            next_cursor: None,
        })
    }

    async fn fetch_issue_comments(
        &self,
        _o: &str,
        _n: &str,
        _num: i64,
        _id: i64,
        _c: Option<String>,
        _p: u32,
    ) -> Result<CommentPage> {
        Ok(CommentPage {
            items: vec![],
            next_cursor: None,
        })
    }

    async fn fetch_user(&self, user: &UserRef) -> Result<UserFetch> {
        // Simulate 404 user
        Ok(UserFetch::Missing(collector::fetcher::MissingUser {
            id: user.id,
            login: user.login.clone(),
            status: Some(StatusCode::NOT_FOUND),
        }))
    }
}

#[tokio::test]
async fn user_404_marks_found_false() -> Result<()> {
    let fixture = match DbFixture::from_env() {
        Ok(f) => f,
        Err(err) => {
            eprintln!("skipping user_404_marks_found_false: {err}");
            return Ok(());
        }
    };
    let handle = fixture.create("user_missing").await?;
    let db = Arc::new(PgDatabase::connect(handle.database_url()).await?);
    let repos: Arc<dyn Repositories> = db.clone();

    db.collection_jobs()
        .create(CollectionJobCreate {
            owner: "o".into(),
            name: "r".into(),
            priority: 0,
        })
        .await?;
    let cfgv = cfg();
    let collector = Collector::new(
        cfgv.clone(),
        Arc::new(MissingUserFetcher),
        repos,
        cfgv.max_concurrent_repos,
    );
    collector.run_once().await?;

    // Verify user persisted with found=false
    let row = db
        .users()
        .get_by_login("ghost-404")
        .await?
        .expect("user row");
    assert!(!row.found);

    handle.cleanup().await?;
    Ok(())
}

struct MissingIssueCommentsFetcher;

#[async_trait]
impl DataFetcher for MissingIssueCommentsFetcher {
    async fn fetch_repo(&self, owner: &str, name: &str) -> Result<RepoSnapshot> {
        Ok(RepoSnapshot {
            repository: NormalizedRepository {
                id: 9010,
                full_name: format!("{}/{}", owner, name),
                is_fork: false,
                created_at: Utc::now(),
                pushed_at: None,
                raw: json!({}),
            },
        })
    }

    async fn fetch_issues(
        &self,
        _owner: &str,
        _name: &str,
        repo_id: i64,
        _since: Option<chrono::DateTime<Utc>>,
        _cursor: Option<String>,
        _per_page: u32,
    ) -> Result<IssuePage> {
        let issue = NormalizedIssue {
            id: 9011,
            repo_id,
            number: 99,
            is_pull_request: false,
            state: "open".into(),
            title: "t".into(),
            body: None,
            user_id: None,
            comments_count: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            closed_at: None,
            dedupe_hash: "hh".into(),
            raw: json!({}),
        };
        Ok(IssuePage {
            items: vec![IssueRecord {
                issue,
                author: None,
            }],
            next_cursor: None,
        })
    }

    async fn fetch_issue_comments(
        &self,
        owner: &str,
        name: &str,
        number: i64,
        _issue_id: i64,
        _cursor: Option<String>,
        _per_page: u32,
    ) -> Result<CommentPage> {
        // Simulate 404 when listing comments for the issue -> collector should mark issue found=false
        Err(anyhow!(collector::client::GithubApiError::status(
            StatusCode::NOT_FOUND,
            format!("repos/{}/{}/issues/{}", owner, name, number)
        )))
    }

    async fn fetch_user(&self, _user: &UserRef) -> Result<UserFetch> {
        Ok(UserFetch::Missing(collector::fetcher::MissingUser {
            id: 0,
            login: "ghost".into(),
            status: None,
        }))
    }
}

#[tokio::test]
async fn issue_comments_404_marks_issue_found_false() -> Result<()> {
    let fixture = match DbFixture::from_env() {
        Ok(f) => f,
        Err(err) => {
            eprintln!("skipping issue_comments_404_marks_issue_found_false: {err}");
            return Ok(());
        }
    };
    let handle = fixture.create("issue_missing").await?;
    let db = Arc::new(PgDatabase::connect(handle.database_url()).await?);
    let repos: Arc<dyn Repositories> = db.clone();

    db.collection_jobs()
        .create(CollectionJobCreate {
            owner: "o".into(),
            name: "r".into(),
            priority: 0,
        })
        .await?;
    let cfgv = cfg();
    let collector = Collector::new(
        cfgv.clone(),
        Arc::new(MissingIssueCommentsFetcher),
        repos.clone(),
        cfgv.max_concurrent_repos,
    );
    collector.run_once().await?;

    // Verify issue row exists with found=false
    let rows = db
        .issues()
        .query(IssueQuery {
            repo_full_name: Some("o/r".into()),
            limit: Some(10),
            spam: None,
            since: None,
        })
        .await?;
    let issue = rows
        .into_iter()
        .find(|i| i.number == 99)
        .expect("issue row");
    assert!(!issue.found);

    handle.cleanup().await?;
    Ok(())
}
