use std::sync::Arc;

use axum::body::to_bytes;
use axum::{http::Request, Router};
use chrono::Utc;
use db::models::{CollectionJobRow, CollectionStatus};
use db::repositories::*;
use serde_json::Value;
use sqlx::PgPool;
use tower::util::ServiceExt;

use api::{build_router, routes::ApiState};

// --- Test doubles for repository traits ---

#[derive(Clone, Default)]
struct NoopRepo;

#[async_trait::async_trait]
impl RepoRepository for NoopRepo {
    async fn upsert(&self, _repo: db::models::RepositoryRow) -> db::errors::Result<()> {
        panic!("unused")
    }
    async fn get_by_full_name(
        &self,
        _full_name: &str,
    ) -> db::errors::Result<Option<db::models::RepositoryRow>> {
        panic!("unused")
    }
    async fn list(&self, _limit: i64) -> db::errors::Result<Vec<db::models::RepositoryRow>> {
        panic!("unused")
    }
}

#[async_trait::async_trait]
impl UserRepository for NoopRepo {
    async fn upsert(&self, _user: db::models::UserRow) -> db::errors::Result<()> {
        panic!("unused")
    }
    async fn get_by_id(&self, _id: i64) -> db::errors::Result<Option<db::models::UserRow>> {
        panic!("unused")
    }
    async fn get_by_login(&self, _login: &str) -> db::errors::Result<Option<db::models::UserRow>> {
        panic!("unused")
    }
}

#[async_trait::async_trait]
impl IssueRepository for NoopRepo {
    async fn upsert(&self, _issue: db::models::IssueRow) -> db::errors::Result<()> {
        panic!("unused")
    }
    async fn query(
        &self,
        _query: db::models::IssueQuery,
    ) -> db::errors::Result<Vec<db::models::IssueRow>> {
        panic!("unused")
    }
    async fn list_by_repo(
        &self,
        _repo_id: i64,
        _since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> db::errors::Result<Vec<db::models::IssueRow>> {
        panic!("unused")
    }
}

#[async_trait::async_trait]
impl CommentRepository for NoopRepo {
    async fn upsert(&self, _comment: db::models::CommentRow) -> db::errors::Result<()> {
        panic!("unused")
    }
    async fn list_by_issue(
        &self,
        _issue_id: i64,
    ) -> db::errors::Result<Vec<db::models::CommentRow>> {
        panic!("unused")
    }
}

#[async_trait::async_trait]
impl WatermarkRepository for NoopRepo {
    async fn get(
        &self,
        _repo_full_name: &str,
    ) -> db::errors::Result<Option<db::models::CollectorWatermarkRow>> {
        panic!("unused")
    }
    async fn set(&self, _watermark: db::models::WatermarkUpdate) -> db::errors::Result<()> {
        panic!("unused")
    }
}

#[async_trait::async_trait]
impl SpamFlagsRepository for NoopRepo {
    async fn upsert(&self, _flag: db::models::SpamFlagUpsert) -> db::errors::Result<()> {
        panic!("unused")
    }
    async fn list_for_subject(
        &self,
        _subject_type: &str,
        _subject_id: i64,
    ) -> db::errors::Result<Vec<db::models::SpamFlagRow>> {
        panic!("unused")
    }
    async fn top_spammy_users(
        &self,
        _since: Option<chrono::DateTime<chrono::Utc>>,
        _limit: i64,
    ) -> db::errors::Result<Vec<db::models::ActorSpamSummary>> {
        panic!("unused")
    }
}

#[derive(Clone)]
struct TestCollectionJobRepo {
    jobs: Vec<CollectionJobRow>,
}

#[async_trait::async_trait]
impl CollectionJobRepository for TestCollectionJobRepo {
    async fn create(
        &self,
        _job: db::models::CollectionJobCreate,
    ) -> db::errors::Result<CollectionJobRow> {
        panic!("unused")
    }
    async fn get_pending(&self, _limit: i32) -> db::errors::Result<Vec<CollectionJobRow>> {
        panic!("unused")
    }
    async fn mark_in_progress(&self, _id: i64) -> db::errors::Result<()> {
        panic!("unused")
    }
    async fn update(&self, _update: db::models::CollectionJobUpdate) -> db::errors::Result<()> {
        panic!("unused")
    }
    async fn list(&self, _limit: i32) -> db::errors::Result<Vec<CollectionJobRow>> {
        Ok(self.jobs.clone())
    }
}

#[derive(Clone)]
struct TestRepos {
    repos: NoopRepo,
    users: NoopRepo,
    issues: NoopRepo,
    comments: NoopRepo,
    watermarks: NoopRepo,
    spam: NoopRepo,
    jobs: TestCollectionJobRepo,
}

impl Repositories for TestRepos {
    fn repos(&self) -> &dyn RepoRepository {
        &self.repos
    }
    fn users(&self) -> &dyn UserRepository {
        &self.users
    }
    fn issues(&self) -> &dyn IssueRepository {
        &self.issues
    }
    fn comments(&self) -> &dyn CommentRepository {
        &self.comments
    }
    fn watermarks(&self) -> &dyn WatermarkRepository {
        &self.watermarks
    }
    fn spam_flags(&self) -> &dyn SpamFlagsRepository {
        &self.spam
    }
    fn collection_jobs(&self) -> &dyn CollectionJobRepository {
        &self.jobs
    }
}

fn mk_job(id: i64, status: CollectionStatus, err: Option<&str>) -> CollectionJobRow {
    let now = Utc::now();
    CollectionJobRow {
        id,
        owner: "o".into(),
        name: format!("r{}", id),
        full_name: format!("o/r{}", id),
        status: status.clone(),
        priority: 10,
        last_attempt_at: Some(now),
        last_completed_at: if matches!(status, CollectionStatus::Completed) {
            Some(now)
        } else {
            None
        },
        failure_count: if matches!(status, CollectionStatus::Completed) {
            0
        } else {
            1
        },
        error_message: err.map(|s| s.to_string()),
        created_at: now,
        updated_at: now,
    }
}

async fn setup_app(jobs: Vec<CollectionJobRow>) -> Router {
    let repos: Arc<dyn Repositories> = Arc::new(TestRepos {
        repos: NoopRepo,
        users: NoopRepo,
        issues: NoopRepo,
        comments: NoopRepo,
        watermarks: NoopRepo,
        spam: NoopRepo,
        jobs: TestCollectionJobRepo { jobs },
    });

    let pool = PgPool::connect_lazy("postgres://postgres:postgres@localhost:5432/github_spam")
        .expect("lazy pool");
    let state = Arc::new(ApiState {
        repositories: repos,
        metrics_path: "/metrics",
        pool: Arc::new(pool),
        config: common::config::AppConfig {
            database: common::config::DatabaseConfig {
                url: "postgres://postgres:postgres@localhost:5432/github_spam".to_string(),
                test_admin_url: None,
            },
            github: common::config::GithubConfig {
                tokens: Vec::new(),
                token_ids: Vec::new(),
                token_secrets: Vec::new(),
                user_agent: "test-agent".to_string(),
            },
            collector: common::config::CollectorConfig {
                interval_secs: 60,
                page_size: 100,
                run_once: false,
                fetch_mode: common::config::FetchMode::Hybrid,
                max_concurrent_repos: 4,
            },
            broker: common::config::BrokerConfig {
                max_inflight: 32,
                per_repo_inflight: 2,
                distributed: false,
                cache_capacity: 5000,
                cache_ttl_secs: 600,
                backoff_base_ms: 500,
                backoff_max_ms: 60_000,
                jitter_frac: 0.2,
                weights: std::collections::HashMap::new(),
                queue_bounds: std::collections::HashMap::new(),
            },
            api: common::config::ApiConfig {
                bind: "0.0.0.0:3000".to_string(),
            },
            observability: common::config::ObservabilityConfig {
                metrics_path: "/metrics".to_string(),
                metrics_bind: "0.0.0.0:9091".to_string(),
            },
        },
    });
    build_router(state)
}

#[tokio::test]
async fn collection_jobs_includes_error_and_timestamps() {
    let long = "x".repeat(1000);
    let jobs = vec![
        mk_job(1, CollectionStatus::Completed, Some("should be hidden")),
        mk_job(2, CollectionStatus::Failed, Some("network timeout")),
        mk_job(3, CollectionStatus::Error, Some(&long)),
    ];
    let app = setup_app(jobs).await;

    let res = app
        .oneshot(
            Request::get("/collection-jobs?limit=10")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(res.status().is_success());
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    let arr = v.as_array().expect("array");
    assert_eq!(arr.len(), 3);

    // Completed: error_message must be null; last_completed_at present
    let j1 = &arr[0];
    assert_eq!(j1.get("status").unwrap().as_str().unwrap(), "Completed");
    assert!(j1.get("error_message").unwrap().is_null());
    assert!(j1.get("last_completed_at").is_some());

    // Failed: error_message shown
    let j2 = &arr[1];
    assert_eq!(j2.get("status").unwrap().as_str().unwrap(), "Failed");
    assert_eq!(
        j2.get("error_message").unwrap().as_str().unwrap(),
        "network timeout"
    );

    // Error: error_message truncated with ellipsis
    let j3 = &arr[2];
    assert_eq!(j3.get("status").unwrap().as_str().unwrap(), "Error");
    let em = j3.get("error_message").unwrap().as_str().unwrap();
    assert!(em.ends_with('…'));
    assert_eq!(em.chars().count(), 513);
}
