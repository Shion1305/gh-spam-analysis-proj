use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use collector::fetcher::{CommentPage, DataFetcher, IssuePage, RepoSnapshot, UserFetch};
use collector::service::Collector;
use common::config::{CollectorConfig, FetchMode};
use db::models::{CollectionJobCreate, CollectionStatus};
use db::pg::PgDatabase;
use db::Repositories;
use db_test_fixture::DbFixture;
use http::StatusCode;
use normalizer::models::NormalizedRepository;
use normalizer::payloads::UserRef;
use serde_json::json;

#[derive(Clone, Copy)]
enum Mode {
    Ok,
    Transient,
    Permanent,
}

struct StubFetcher { mode: Mode }

#[async_trait]
impl DataFetcher for StubFetcher {
    async fn fetch_repo(&self, owner: &str, name: &str) -> Result<RepoSnapshot> {
        match self.mode {
            Mode::Ok => Ok(RepoSnapshot {
                repository: NormalizedRepository {
                    id: 1,
                    full_name: format!("{}/{}", owner, name),
                    is_fork: false,
                    created_at: chrono::Utc::now(),
                    pushed_at: None,
                    raw: json!({"id":1}),
                },
            }),
            Mode::Transient => Err(anyhow!(gh_broker::HttpStatusError::new(
                StatusCode::INTERNAL_SERVER_ERROR
            ))),
            Mode::Permanent => Err(anyhow!(collector::client::GithubApiError::status(
                StatusCode::NOT_FOUND,
                format!("repos/{owner}/{name}"),
            ))),
        }
    }

    async fn fetch_issues(
        &self,
        _owner: &str,
        _name: &str,
        _repo_id: i64,
        _since: Option<chrono::DateTime<chrono::Utc>>,
        _cursor: Option<String>,
        _per_page: u32,
    ) -> Result<IssuePage> {
        Ok(IssuePage { items: vec![], next_cursor: None })
    }

    async fn fetch_issue_comments(
        &self,
        _owner: &str,
        _name: &str,
        _issue_number: i64,
        _issue_id: i64,
        _cursor: Option<String>,
        _per_page: u32,
    ) -> Result<CommentPage> {
        Ok(CommentPage { items: vec![], next_cursor: None })
    }

    async fn fetch_user(&self, _user: &UserRef) -> Result<UserFetch> {
        Ok(UserFetch::Missing(collector::fetcher::MissingUser { id: 0, login: "ghost".into(), status: None }))
    }
}

fn cfg() -> CollectorConfig {
    CollectorConfig { page_size: 50, interval_secs: 1, run_once: true, fetch_mode: FetchMode::Hybrid }
}

#[tokio::test]
async fn job_completes_on_success() -> Result<()> {
    let fixture = match DbFixture::from_env() {
        Ok(f) => f,
        Err(err) => {
            eprintln!("skipping job_completes_on_success: {err}");
            return Ok(());
        }
    };
    let handle = fixture.create("job_ok").await?;
    let db = Arc::new(PgDatabase::connect(handle.database_url()).await?);
    let repos: Arc<dyn Repositories> = db.clone();

    let job = db
        .collection_jobs()
        .create(CollectionJobCreate { owner: "o".into(), name: "r".into(), priority: 0 })
        .await?;

    let fetcher: Arc<dyn DataFetcher> = Arc::new(StubFetcher { mode: Mode::Ok });
    let collector = Collector::new(cfg(), fetcher, repos);
    collector.run_once().await?;

    let listed = db.collection_jobs().list(10).await?;
    let j = listed.into_iter().find(|j| j.id == job.id).unwrap();
    assert!(matches!(j.status, CollectionStatus::Completed));

    handle.cleanup().await?;
    Ok(())
}

#[tokio::test]
async fn job_retries_on_transient_error() -> Result<()> {
    let fixture = match DbFixture::from_env() {
        Ok(f) => f,
        Err(err) => {
            eprintln!("skipping job_retries_on_transient_error: {err}");
            return Ok(());
        }
    };
    let handle = fixture.create("job_retry").await?;
    let db = Arc::new(PgDatabase::connect(handle.database_url()).await?);
    let repos: Arc<dyn Repositories> = db.clone();

    let job = db
        .collection_jobs()
        .create(CollectionJobCreate { owner: "o".into(), name: "r".into(), priority: 0 })
        .await?;

    let fetcher: Arc<dyn DataFetcher> = Arc::new(StubFetcher { mode: Mode::Transient });
    let collector = Collector::new(cfg(), fetcher, repos);
    collector.run_once().await?;

    let listed = db.collection_jobs().list(10).await?;
    let j = listed.into_iter().find(|j| j.id == job.id).unwrap();
    // Transient: returns to pending with failure count incremented
    assert!(matches!(j.status, CollectionStatus::Pending));
    assert!(j.failure_count >= 1);

    handle.cleanup().await?;
    Ok(())
}

#[tokio::test]
async fn job_errors_on_permanent_error() -> Result<()> {
    let fixture = match DbFixture::from_env() {
        Ok(f) => f,
        Err(err) => {
            eprintln!("skipping job_errors_on_permanent_error: {err}");
            return Ok(());
        }
    };
    let handle = fixture.create("job_perm").await?;
    let db = Arc::new(PgDatabase::connect(handle.database_url()).await?);
    let repos: Arc<dyn Repositories> = db.clone();

    let job = db
        .collection_jobs()
        .create(CollectionJobCreate { owner: "o".into(), name: "r".into(), priority: 0 })
        .await?;

    let fetcher: Arc<dyn DataFetcher> = Arc::new(StubFetcher { mode: Mode::Permanent });
    let collector = Collector::new(cfg(), fetcher, repos);
    collector.run_once().await?;

    let listed = db.collection_jobs().list(10).await?;
    let j = listed.into_iter().find(|j| j.id == job.id).unwrap();
    assert!(matches!(j.status, CollectionStatus::Error));

    handle.cleanup().await?;
    Ok(())
}
