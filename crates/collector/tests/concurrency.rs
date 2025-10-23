use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use collector::fetcher::{CommentPage, DataFetcher, IssuePage, RepoSnapshot, UserFetch};
use collector::service::Collector;
use common::config::{CollectorConfig, FetchMode};
use db::models::CollectionJobCreate;
use db::pg::PgDatabase;
use db::Repositories;
use db_test_fixture::DbFixture;
use normalizer::models::NormalizedRepository;
use normalizer::payloads::UserRef;
use serde_json::json;
use tokio::time::Instant;

struct SleepyFetcher {
    sleep_ms: u64,
    active: Arc<std::sync::atomic::AtomicUsize>,
    max_active: Arc<std::sync::atomic::AtomicUsize>,
}

impl SleepyFetcher {
    fn new(sleep_ms: u64) -> Self {
        Self {
            sleep_ms,
            active: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            max_active: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    fn inc(&self) {
        let cur = self.active.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        loop {
            let max = self
                .max_active
                .load(std::sync::atomic::Ordering::Relaxed);
            if cur <= max {
                break;
            }
            if self
                .max_active
                .compare_exchange(
                    max,
                    cur,
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                )
                .is_ok()
            {
                break;
            }
        }
    }

    fn dec(&self) {
        self.active.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[async_trait]
impl DataFetcher for SleepyFetcher {
    async fn fetch_repo(&self, owner: &str, name: &str) -> Result<RepoSnapshot> {
        self.inc();
        tokio::time::sleep(Duration::from_millis(self.sleep_ms)).await;
        self.dec();
        Ok(RepoSnapshot {
            repository: NormalizedRepository {
                id: 42,
                full_name: format!("{}/{}", owner, name),
                is_fork: false,
                created_at: Utc::now(),
                pushed_at: None,
                raw: json!({"id":42}),
            },
        })
    }

    async fn fetch_issues(
        &self,
        _owner: &str,
        _name: &str,
        _repo_id: i64,
        _since: Option<chrono::DateTime<Utc>>,
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

#[tokio::test]
async fn processes_repositories_in_parallel() -> Result<()> {
    let fixture = match DbFixture::from_env() {
        Ok(f) => f,
        Err(err) => {
            eprintln!("skipping processes_repositories_in_parallel: {err}");
            return Ok(());
        }
    };
    let handle = fixture.create("conc").await?;
    let db = Arc::new(PgDatabase::connect(handle.database_url()).await?);
    let repos: Arc<dyn Repositories> = db.clone();

    // Create 3 jobs
    for i in 0..3 {
        db.collection_jobs()
            .create(CollectionJobCreate { owner: format!("o{i}"), name: format!("r{i}"), priority: 0 })
            .await?;
    }

    let fetcher = Arc::new(SleepyFetcher::new(250));
    let cfg = CollectorConfig {
        page_size: 50,
        interval_secs: 0,
        run_once: true,
        fetch_mode: FetchMode::Hybrid,
        max_concurrent_repos: 2,
    };
    let collector = Collector::new(cfg, fetcher.clone(), repos);

    let start = Instant::now();
    collector.run_once().await?;
    let elapsed = start.elapsed();

    let max_active = fetcher
        .max_active
        .load(std::sync::atomic::Ordering::Relaxed);

    // With 3 jobs, 250ms each, concurrency=2: expect < 600ms and >= 250ms, and at least 2 in-flight.
    assert!(max_active >= 2, "expected >=2 concurrent, got {}", max_active);
    assert!(elapsed.as_millis() < 600, "took too long: {:?}", elapsed);
    assert!(elapsed.as_millis() >= 250, "too fast (unexpected): {:?}", elapsed);

    handle.cleanup().await?;
    Ok(())
}

