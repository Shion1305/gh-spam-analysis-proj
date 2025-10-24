use std::sync::Arc;

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

struct MismatchFetcher;

#[async_trait]
impl DataFetcher for MismatchFetcher {
    async fn fetch_repo(&self, _owner: &str, _name: &str) -> Result<RepoSnapshot> {
        // Intentionally return a different repo full_name than the requested seed
        Ok(RepoSnapshot {
            repository: NormalizedRepository {
                id: 999_001,
                full_name: "octocat/Spoon-Knife".into(),
                is_fork: false,
                created_at: Utc::now(),
                pushed_at: None,
                raw: json!({"id": 999_001}),
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
        // Not reached due to mismatch guard
        Ok(IssuePage {
            items: Vec::new(),
            next_cursor: None,
        })
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
        // Not reached due to mismatch guard
        Ok(CommentPage {
            items: Vec::new(),
            next_cursor: None,
        })
    }

    async fn fetch_user(&self, user: &UserRef) -> Result<UserFetch> {
        Ok(UserFetch::Missing(collector::fetcher::MissingUser {
            id: user.id,
            login: user.login.clone(),
            status: None,
        }))
    }
}

fn test_config() -> CollectorConfig {
    CollectorConfig {
        page_size: 10,
        interval_secs: 1,
        run_once: true,
        fetch_mode: FetchMode::Rest,
        max_concurrent_repos: 1,
    }
}

#[tokio::test]
async fn seed_repo_mismatch_is_guarded_and_marked_error() -> Result<()> {
    let fixture = match DbFixture::from_env() {
        Ok(f) => f,
        Err(err) => {
            eprintln!("skipping seed_repo_mismatch_is_guarded_and_marked_error: {err}");
            return Ok(());
        }
    };
    let handle = fixture.create("collector_seed_mismatch").await?;
    let db = PgDatabase::connect(handle.database_url()).await?;

    // Create a pending job for octocat/Hello-World
    let job = db
        .collection_jobs()
        .create(CollectionJobCreate {
            owner: "octocat".into(),
            name: "Hello-World".into(),
            priority: 0,
        })
        .await?;

    // Run collector with mismatching fetcher
    let fetcher = Arc::new(MismatchFetcher) as Arc<dyn DataFetcher>;
    let repos: Arc<dyn Repositories> = Arc::new(db.clone());
    let cfg = test_config();
    let collector = Collector::new(
        cfg.clone(),
        fetcher,
        repos.clone(),
        cfg.max_concurrent_repos,
    );
    let _ = collector.run_once().await; // expect error path for the job

    // Verify job is marked as Error (permanent) and has a helpful message
    let row: (String, Option<String>) =
        sqlx::query_as("SELECT status::text, error_message FROM collection_jobs WHERE id = $1")
            .bind(job.id)
            .fetch_one(db.pool())
            .await?;
    assert_eq!(row.0, "error");
    if let Some(msg) = row.1 {
        assert!(msg.to_lowercase().contains("seed mismatch"));
    } else {
        panic!("expected error_message to be set");
    }

    // Ensure we did not upsert any repository due to the guard
    let repo_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM repositories")
        .fetch_one(db.pool())
        .await?;
    assert_eq!(repo_count, 0);

    handle.cleanup().await?;
    Ok(())
}
