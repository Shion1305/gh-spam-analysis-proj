use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use collector::fetcher::{
    CommentPage, CommentRecord, DataFetcher, IssuePage, IssueRecord, RepoSnapshot, UserFetch,
};
use collector::service::Collector;
use common::config::{CollectorConfig, FetchMode};
use db::models::{CollectionJobCreate, CollectionJobUpdate, CollectionStatus};
use db::pg::PgDatabase;
use db::Repositories;
use db_test_fixture::DbFixture;
use normalizer::models::{
    NormalizedComment, NormalizedIssue, NormalizedRepository, NormalizedUser,
};
use normalizer::payloads::UserRef;
use serde_json::json;

struct StubFetcher {
    repo_id: i64,
    full_name: String,
}

#[async_trait]
impl DataFetcher for StubFetcher {
    async fn fetch_repo(&self, _owner: &str, _name: &str) -> Result<RepoSnapshot> {
        Ok(RepoSnapshot {
            repository: NormalizedRepository {
                id: self.repo_id,
                full_name: self.full_name.clone(),
                is_fork: false,
                created_at: Utc::now(),
                pushed_at: None,
                raw: json!({"id": self.repo_id}),
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
        Ok(IssuePage {
            items: vec![],
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
        Ok(CommentPage {
            items: vec![],
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
        page_size: 50,
        interval_secs: 1,
        run_once: true,
        fetch_mode: FetchMode::Rest,
    }
}

#[tokio::test]
async fn collector_dedup_repo_registration_across_runs() -> Result<()> {
    let fixture = match DbFixture::from_env() {
        Ok(f) => f,
        Err(err) => {
            eprintln!("skipping collector_dedup_repo_registration_across_runs: {err}");
            return Ok(());
        }
    };
    let handle = fixture.create("collector_dedup").await?;
    let db = PgDatabase::connect(handle.database_url()).await?;
    let database = Arc::new(db);
    let repos: Arc<dyn Repositories> = database.clone();

    // Create a pending job
    let job = database
        .collection_jobs()
        .create(CollectionJobCreate {
            owner: "Owner".into(),
            name: "Example".into(),
            priority: 0,
        })
        .await?;

    // First run: register repo with mixed-case full_name
    let fetcher1 = Arc::new(StubFetcher {
        repo_id: 4242,
        full_name: "Owner/Example".into(),
    }) as Arc<dyn DataFetcher>;
    let collector1 = Collector::new(test_config(), fetcher1, repos.clone());
    collector1.run_once().await?;

    let count1: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM repositories")
        .fetch_one(database.pool())
        .await?;
    assert_eq!(count1, 1);

    // Reset job to pending to simulate re-processing
    database
        .collection_jobs()
        .update(CollectionJobUpdate {
            id: job.id,
            status: CollectionStatus::Pending,
            error_message: None,
        })
        .await?;

    // Second run: same repo id, different full_name case
    let fetcher2 = Arc::new(StubFetcher {
        repo_id: 4242,
        full_name: "owner/example".into(),
    }) as Arc<dyn DataFetcher>;
    let collector2 = Collector::new(test_config(), fetcher2, repos.clone());
    collector2.run_once().await?;

    let (count2, full_name): (i64, String) =
        sqlx::query_as("SELECT COUNT(*), MIN(full_name) FROM repositories")
            .fetch_one(database.pool())
            .await?;
    assert_eq!(count2, 1, "still a single repository row after second run");
    assert_eq!(
        full_name, "owner/example",
        "full_name updated using latest payload"
    );

    handle.cleanup().await?;
    Ok(())
}
