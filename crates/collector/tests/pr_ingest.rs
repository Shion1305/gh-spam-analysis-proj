use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use collector::fetcher::{
    CommentPage, DataFetcher, IssuePage, IssueRecord, RepoSnapshot, UserFetch,
};
use collector::service::Collector;
use common::config::{CollectorConfig, FetchMode};
use db::models::CollectionJobCreate;
use db::pg::PgDatabase;
use db::Repositories;
use db_test_fixture::DbFixture;
use normalizer::models::{NormalizedIssue, NormalizedRepository};
use normalizer::payloads::UserRef;
use serde_json::json;

struct PrIssueFetcher;

#[async_trait]
impl DataFetcher for PrIssueFetcher {
    async fn fetch_repo(&self, owner: &str, name: &str) -> Result<RepoSnapshot> {
        Ok(RepoSnapshot {
            repository: NormalizedRepository {
                id: 777,
                full_name: format!("{}/{}", owner, name),
                is_fork: false,
                created_at: Utc::now(),
                pushed_at: None,
                raw: json!({"id":777}),
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
            id: 1,
            repo_id,
            number: 10,
            is_pull_request: false,
            state: "open".into(),
            title: "Issue".into(),
            body: Some("body".into()),
            user_id: None,
            comments_count: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            closed_at: None,
            dedupe_hash: "hash-issue".into(),
            raw: json!({}),
        };
        let pr = NormalizedIssue {
            id: 2,
            repo_id,
            number: 11,
            is_pull_request: true,
            state: "open".into(),
            title: "PR".into(),
            body: Some("pr body".into()),
            user_id: None,
            comments_count: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            closed_at: None,
            dedupe_hash: "hash-pr".into(),
            raw: json!({}),
        };
        Ok(IssuePage {
            items: vec![
                IssueRecord {
                    issue,
                    author: None,
                },
                IssueRecord {
                    issue: pr,
                    author: None,
                },
            ],
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

#[tokio::test]
async fn ingests_prs_and_issues() -> Result<()> {
    let fixture = match DbFixture::from_env() {
        Ok(f) => f,
        Err(err) => {
            eprintln!("skipping ingests_prs_and_issues: {err}");
            return Ok(());
        }
    };
    let handle = fixture.create("pr_ingest").await?;
    let db = Arc::new(PgDatabase::connect(handle.database_url()).await?);
    let repos: Arc<dyn Repositories> = db.clone();

    db.collection_jobs()
        .create(CollectionJobCreate {
            owner: "o".into(),
            name: "r".into(),
            priority: 0,
        })
        .await?;

    let cfg = CollectorConfig {
        page_size: 50,
        interval_secs: 0,
        run_once: true,
        fetch_mode: FetchMode::Hybrid,
        max_concurrent_repos: 1,
    };
    let collector = Collector::new(cfg, Arc::new(PrIssueFetcher), repos);
    collector.run_once().await?;

    // Assert both records exist and PR flag is correct
    let rows = db
        .issues()
        .query(db::models::IssueQuery {
            repo_full_name: Some("o/r".into()),
            limit: Some(100),
            spam: None,
            since: None,
        })
        .await?;
    let mut seen_issue = false;
    let mut seen_pr = false;
    for row in rows {
        if row.number == 10 {
            assert!(!row.is_pull_request);
            seen_issue = true;
        }
        if row.number == 11 {
            assert!(row.is_pull_request);
            seen_pr = true;
        }
    }
    assert!(seen_issue && seen_pr);

    handle.cleanup().await?;
    Ok(())
}
