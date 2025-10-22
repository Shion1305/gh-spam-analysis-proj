use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, QueryBuilder};
use tokio::time::{sleep, Duration};
use tracing::{instrument, warn};

use crate::errors::{DbError, Result};
use crate::models::{
    ActorSpamSummary, CollectionJobCreate, CollectionJobRow, CollectionJobUpdate, CollectionStatus,
    CollectorWatermarkRow, CommentRow, IssueQuery, IssueRow, RepositoryRow, SpamFlagRow,
    SpamFlagUpsert, UserRow, WatermarkUpdate,
};
use crate::repositories::{
    CollectionJobRepository, CommentRepository, IssueRepository, RepoRepository, Repositories,
    SpamFlagsRepository, UserRepository, WatermarkRepository,
};

pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("../../migrations")
        .run(pool)
        .await
        .map_err(DbError::Migration)
}

#[derive(Clone)]
pub struct PgDatabase {
    pool: PgPool,
    repo_repo: Arc<PgRepoRepository>,
    user_repo: Arc<PgUserRepository>,
    issue_repo: Arc<PgIssueRepository>,
    comment_repo: Arc<PgCommentRepository>,
    watermark_repo: Arc<PgWatermarkRepository>,
    spam_repo: Arc<PgSpamFlagsRepository>,
    collection_job_repo: Arc<PgCollectionJobRepository>,
}

impl PgDatabase {
    pub async fn connect(database_url: &str) -> Result<Self> {
        const MAX_ATTEMPTS: u32 = 5;
        const BASE_DELAY_MS: u64 = 500;

        let mut attempts = 0;
        loop {
            match PgPoolOptions::new()
                .max_connections(10)
                .connect(database_url)
                .await
            {
                Ok(pool) => {
                    run_migrations(&pool).await?;
                    return Ok(Self::from_pool(pool));
                }
                Err(err) => {
                    attempts += 1;
                    if attempts >= MAX_ATTEMPTS {
                        return Err(DbError::Query(err));
                    }

                    let exp = (attempts - 1).min(5);
                    let backoff = Duration::from_millis(BASE_DELAY_MS * (1u64 << exp));
                    warn!(
                        attempts,
                        error = %err,
                        wait_ms = backoff.as_millis(),
                        "database connection failed; retrying"
                    );
                    sleep(backoff).await;
                }
            }
        }
    }

    pub fn from_pool(pool: PgPool) -> Self {
        let repo_repo = Arc::new(PgRepoRepository { pool: pool.clone() });
        let user_repo = Arc::new(PgUserRepository { pool: pool.clone() });
        let issue_repo = Arc::new(PgIssueRepository { pool: pool.clone() });
        let comment_repo = Arc::new(PgCommentRepository { pool: pool.clone() });
        let watermark_repo = Arc::new(PgWatermarkRepository { pool: pool.clone() });
        let spam_repo = Arc::new(PgSpamFlagsRepository { pool: pool.clone() });
        let collection_job_repo = Arc::new(PgCollectionJobRepository { pool: pool.clone() });

        Self {
            pool,
            repo_repo,
            user_repo,
            issue_repo,
            comment_repo,
            watermark_repo,
            spam_repo,
            collection_job_repo,
        }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

impl Repositories for PgDatabase {
    fn repos(&self) -> &dyn RepoRepository {
        &*self.repo_repo
    }

    fn users(&self) -> &dyn UserRepository {
        &*self.user_repo
    }

    fn issues(&self) -> &dyn IssueRepository {
        &*self.issue_repo
    }

    fn comments(&self) -> &dyn CommentRepository {
        &*self.comment_repo
    }

    fn watermarks(&self) -> &dyn WatermarkRepository {
        &*self.watermark_repo
    }

    fn spam_flags(&self) -> &dyn SpamFlagsRepository {
        &*self.spam_repo
    }

    fn collection_jobs(&self) -> &dyn CollectionJobRepository {
        &*self.collection_job_repo
    }
}

#[derive(Clone)]
struct PgRepoRepository {
    pool: PgPool,
}

#[async_trait]
impl RepoRepository for PgRepoRepository {
    #[instrument(skip(self, repo), fields(full_name = %repo.full_name))]
    async fn upsert(&self, repo: RepositoryRow) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO repositories (id, full_name, is_fork, created_at, pushed_at, raw)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (id) DO UPDATE
                SET full_name = EXCLUDED.full_name,
                    is_fork = EXCLUDED.is_fork,
                    created_at = EXCLUDED.created_at,
                    pushed_at = EXCLUDED.pushed_at,
                    raw = EXCLUDED.raw
            "#,
        )
        .bind(repo.id)
        .bind(repo.full_name)
        .bind(repo.is_fork)
        .bind(repo.created_at)
        .bind(repo.pushed_at)
        .bind(repo.raw)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(DbError::Query)
    }

    async fn get_by_full_name(&self, full_name: &str) -> Result<Option<RepositoryRow>> {
        sqlx::query_as::<_, RepositoryRow>(
            r#"
            SELECT id, full_name, is_fork, created_at, pushed_at, raw
            FROM repositories
            WHERE full_name = $1
            "#,
        )
        .bind(full_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn list(&self, limit: i64) -> Result<Vec<RepositoryRow>> {
        sqlx::query_as::<_, RepositoryRow>(
            r#"
            SELECT id, full_name, is_fork, created_at, pushed_at, raw
            FROM repositories
            ORDER BY full_name
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Query)
    }
}

#[derive(Clone)]
struct PgUserRepository {
    pool: PgPool,
}

#[async_trait]
impl UserRepository for PgUserRepository {
    async fn upsert(&self, user: UserRow) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO users (id, login, type, site_admin, created_at, followers, following, public_repos, raw)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (id) DO UPDATE
                SET login = EXCLUDED.login,
                    type = EXCLUDED.type,
                    site_admin = EXCLUDED.site_admin,
                    created_at = EXCLUDED.created_at,
                    followers = EXCLUDED.followers,
                    following = EXCLUDED.following,
                    public_repos = EXCLUDED.public_repos,
                    raw = EXCLUDED.raw
            "#
        )
        .bind(user.id)
        .bind(user.login)
        .bind(user.user_type)
        .bind(user.site_admin)
        .bind(user.created_at)
        .bind(user.followers)
        .bind(user.following)
        .bind(user.public_repos)
        .bind(user.raw)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(DbError::Query)
    }

    async fn get_by_id(&self, id: i64) -> Result<Option<UserRow>> {
        sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, login, type as "user_type", site_admin, created_at, followers, following, public_repos, raw
            FROM users
            WHERE id = $1
            "#
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn get_by_login(&self, login: &str) -> Result<Option<UserRow>> {
        sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, login, type as "user_type", site_admin, created_at, followers, following, public_repos, raw
            FROM users
            WHERE login = $1
            "#
        )
        .bind(login)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)
    }
}

#[derive(Clone)]
struct PgIssueRepository {
    pool: PgPool,
}

#[async_trait]
impl IssueRepository for PgIssueRepository {
    async fn upsert(&self, issue: IssueRow) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO issues (
                id, repo_id, number, is_pull_request, state, title, body, user_id,
                comments_count, created_at, updated_at, closed_at, dedupe_hash, raw
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            ON CONFLICT (id) DO UPDATE
                SET repo_id = EXCLUDED.repo_id,
                    number = EXCLUDED.number,
                    is_pull_request = EXCLUDED.is_pull_request,
                    state = EXCLUDED.state,
                    title = EXCLUDED.title,
                    body = EXCLUDED.body,
                    user_id = EXCLUDED.user_id,
                    comments_count = EXCLUDED.comments_count,
                    created_at = EXCLUDED.created_at,
                    updated_at = EXCLUDED.updated_at,
                    closed_at = EXCLUDED.closed_at,
                    dedupe_hash = EXCLUDED.dedupe_hash,
                    raw = EXCLUDED.raw
            "#,
        )
        .bind(issue.id)
        .bind(issue.repo_id)
        .bind(issue.number)
        .bind(issue.is_pull_request)
        .bind(issue.state)
        .bind(issue.title)
        .bind(issue.body)
        .bind(issue.user_id)
        .bind(issue.comments_count)
        .bind(issue.created_at)
        .bind(issue.updated_at)
        .bind(issue.closed_at)
        .bind(issue.dedupe_hash)
        .bind(issue.raw)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(DbError::Query)
    }

    async fn query(&self, query: IssueQuery) -> Result<Vec<IssueRow>> {
        let mut builder = QueryBuilder::<Postgres>::new(
            r#"
            SELECT id, repo_id, number, is_pull_request, state, title, body, user_id,
                   comments_count, created_at, updated_at, closed_at, dedupe_hash, raw
            FROM issues
            "#,
        );

        let mut has_where = false;

        if let Some(repo) = &query.repo_full_name {
            builder.push(" WHERE repo_id = (SELECT id FROM repositories WHERE full_name = ");
            builder.push_bind(repo);
            builder.push(") ");
            has_where = true;
        }

        if let Some(since) = query.since {
            builder.push(if has_where { " AND" } else { " WHERE" });
            builder.push(" updated_at >= ");
            builder.push_bind(since);
            builder.push(" ");
            has_where = true;
        }

        if let Some(filter) = query.spam {
            match filter {
                crate::models::SpamFilter::Likely => {
                    builder.push(if has_where { " AND" } else { " WHERE" });
                    builder.push(
                        " EXISTS (SELECT 1 FROM spam_flags WHERE subject_type = 'issue' AND subject_id = issues.id AND score >= 2.5)",
                    );
                }
                crate::models::SpamFilter::Suspicious => {
                    builder.push(if has_where { " AND" } else { " WHERE" });
                    builder.push(
                        " EXISTS (SELECT 1 FROM spam_flags WHERE subject_type = 'issue' AND subject_id = issues.id AND score >= 1.0)",
                    );
                }
                crate::models::SpamFilter::All => {}
            }
        }

        builder.push(" ORDER BY updated_at DESC ");

        if let Some(limit) = query.limit {
            builder.push(" LIMIT ");
            builder.push_bind(limit);
        }

        let query = builder.build_query_as::<IssueRow>();
        query.fetch_all(&self.pool).await.map_err(DbError::Query)
    }

    async fn list_by_repo(
        &self,
        repo_id: i64,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<IssueRow>> {
        if let Some(since) = since {
            sqlx::query_as::<_, IssueRow>(
                r#"
                SELECT id, repo_id, number, is_pull_request, state, title, body,
                       user_id, comments_count, created_at, updated_at, closed_at,
                       dedupe_hash, raw
                FROM issues
                WHERE repo_id = $1 AND updated_at >= $2
                ORDER BY updated_at DESC
                "#,
            )
            .bind(repo_id)
            .bind(since)
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
        } else {
            sqlx::query_as::<_, IssueRow>(
                r#"
                SELECT id, repo_id, number, is_pull_request, state, title, body,
                       user_id, comments_count, created_at, updated_at, closed_at,
                       dedupe_hash, raw
                FROM issues
                WHERE repo_id = $1
                ORDER BY updated_at DESC
                "#,
            )
            .bind(repo_id)
            .fetch_all(&self.pool)
            .await
            .map_err(DbError::Query)
        }
    }
}

#[derive(Clone)]
struct PgCommentRepository {
    pool: PgPool,
}

#[async_trait]
impl CommentRepository for PgCommentRepository {
    async fn upsert(&self, comment: CommentRow) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO comments (
                id, issue_id, user_id, body, created_at, updated_at, dedupe_hash, raw
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (id) DO UPDATE
                SET issue_id = EXCLUDED.issue_id,
                    user_id = EXCLUDED.user_id,
                    body = EXCLUDED.body,
                    created_at = EXCLUDED.created_at,
                    updated_at = EXCLUDED.updated_at,
                    dedupe_hash = EXCLUDED.dedupe_hash,
                    raw = EXCLUDED.raw
            "#,
        )
        .bind(comment.id)
        .bind(comment.issue_id)
        .bind(comment.user_id)
        .bind(comment.body)
        .bind(comment.created_at)
        .bind(comment.updated_at)
        .bind(comment.dedupe_hash)
        .bind(comment.raw)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(DbError::Query)
    }

    async fn list_by_issue(&self, issue_id: i64) -> Result<Vec<CommentRow>> {
        sqlx::query_as::<_, CommentRow>(
            r#"
            SELECT id, issue_id, user_id, body, created_at, updated_at, dedupe_hash, raw
            FROM comments
            WHERE issue_id = $1
            ORDER BY created_at
            "#,
        )
        .bind(issue_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Query)
    }
}

#[derive(Clone)]
struct PgWatermarkRepository {
    pool: PgPool,
}

#[async_trait]
impl WatermarkRepository for PgWatermarkRepository {
    async fn get(&self, repo_full_name: &str) -> Result<Option<CollectorWatermarkRow>> {
        sqlx::query_as::<_, CollectorWatermarkRow>(
            r#"
            SELECT repo_full_name, last_updated
            FROM collector_watermarks
            WHERE repo_full_name = $1
            "#,
        )
        .bind(repo_full_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn set(&self, watermark: WatermarkUpdate) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO collector_watermarks (repo_full_name, last_updated)
            VALUES ($1, $2)
            ON CONFLICT (repo_full_name) DO UPDATE
                SET last_updated = EXCLUDED.last_updated
            "#,
        )
        .bind(watermark.repo_full_name)
        .bind(watermark.last_updated)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(DbError::Query)
    }
}

#[derive(Clone)]
struct PgSpamFlagsRepository {
    pool: PgPool,
}

#[async_trait]
impl SpamFlagsRepository for PgSpamFlagsRepository {
    async fn upsert(&self, flag: SpamFlagUpsert) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO spam_flags (subject_type, subject_id, score, reasons, version)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (subject_type, subject_id, version) DO UPDATE
                SET score = EXCLUDED.score,
                    reasons = EXCLUDED.reasons
            "#,
        )
        .bind(flag.subject_type)
        .bind(flag.subject_id)
        .bind(flag.score)
        .bind(flag.reasons)
        .bind(flag.version)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(DbError::Query)
    }

    async fn list_for_subject(
        &self,
        subject_type: &str,
        subject_id: i64,
    ) -> Result<Vec<SpamFlagRow>> {
        sqlx::query_as::<_, SpamFlagRow>(
            r#"
            SELECT id, subject_type, subject_id, score, reasons, version, created_at
            FROM spam_flags
            WHERE subject_type = $1 AND subject_id = $2
            ORDER BY created_at DESC
            "#,
        )
        .bind(subject_type)
        .bind(subject_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn top_spammy_users(
        &self,
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<ActorSpamSummary>> {
        let rows = sqlx::query!(
            r#"
            SELECT
                login,
                AVG(score) AS avg_score,
                SUM(score) AS total_score,
                COUNT(*) AS flag_count,
                ARRAY_AGG(DISTINCT reason) AS reasons
            FROM (
                SELECT
                    COALESCE(u.login, uc.login) AS login,
                    sf.score,
                    sf.created_at,
                    unnest(sf.reasons) AS reason
                FROM spam_flags sf
                LEFT JOIN issues i ON sf.subject_type = 'issue' AND sf.subject_id = i.id
                LEFT JOIN users u ON i.user_id = u.id
                LEFT JOIN comments c ON sf.subject_type = 'comment' AND sf.subject_id = c.id
                LEFT JOIN users uc ON c.user_id = uc.id
                WHERE ($1::timestamptz IS NULL OR sf.created_at >= $1)
            ) flagged
            WHERE login IS NOT NULL
            GROUP BY login
            ORDER BY total_score DESC
            LIMIT $2
            "#,
            since,
            limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Query)?;

        let mut summaries = Vec::new();
        for row in rows {
            if let Some(login) = row.login {
                summaries.push(ActorSpamSummary {
                    login,
                    avg_score: row.avg_score.unwrap_or_default() as f32,
                    total_score: row.total_score.unwrap_or_default(),
                    flag_count: row.flag_count.unwrap_or_default(),
                    reasons: row.reasons.unwrap_or_default(),
                });
            }
        }
        Ok(summaries)
    }
}

#[derive(Clone)]
struct PgCollectionJobRepository {
    pool: PgPool,
}

#[async_trait]
impl CollectionJobRepository for PgCollectionJobRepository {
    async fn create(&self, job: CollectionJobCreate) -> Result<CollectionJobRow> {
        sqlx::query_as::<_, CollectionJobRow>(
            r#"
            INSERT INTO collection_jobs (owner, name, priority)
            VALUES ($1, $2, $3)
            ON CONFLICT (owner, name) DO UPDATE
                SET priority = EXCLUDED.priority,
                    updated_at = now()
            RETURNING id, owner, name, full_name, status as "status: _", priority, 
                      last_attempt_at, last_completed_at, failure_count, error_message,
                      created_at, updated_at
            "#,
        )
        .bind(job.owner)
        .bind(job.name)
        .bind(job.priority)
        .fetch_one(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn get_pending(&self, limit: i32) -> Result<Vec<CollectionJobRow>> {
        sqlx::query_as::<_, CollectionJobRow>(
            r#"
            SELECT id, owner, name, full_name, status as "status: _", priority,
                   last_attempt_at, last_completed_at, failure_count, error_message,
                   created_at, updated_at
            FROM collection_jobs
            WHERE status = 'pending'
            ORDER BY priority DESC, created_at ASC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn mark_in_progress(&self, id: i64) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE collection_jobs
            SET status = 'in_progress',
                last_attempt_at = now(),
                updated_at = now()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(DbError::Query)
    }

    async fn update(&self, update: CollectionJobUpdate) -> Result<()> {
        let status_change = match update.status {
            CollectionStatus::Completed => {
                sqlx::query(
                    r#"
                    UPDATE collection_jobs
                    SET status = $1,
                        last_completed_at = now(),
                        error_message = NULL,
                        failure_count = 0,
                        updated_at = now()
                    WHERE id = $2
                    "#,
                )
                .bind(update.status)
                .bind(update.id)
                .execute(&self.pool)
                .await
            }
            CollectionStatus::Failed => {
                sqlx::query(
                    r#"
                    UPDATE collection_jobs
                    SET status = 'pending',
                        error_message = $1,
                        failure_count = failure_count + 1,
                        updated_at = now()
                    WHERE id = $2
                    "#,
                )
                .bind(update.error_message)
                .bind(update.id)
                .execute(&self.pool)
                .await
            }
            _ => {
                sqlx::query(
                    r#"
                    UPDATE collection_jobs
                    SET status = $1,
                        error_message = $2,
                        updated_at = now()
                    WHERE id = $3
                    "#,
                )
                .bind(update.status)
                .bind(update.error_message)
                .bind(update.id)
                .execute(&self.pool)
                .await
            }
        };

        status_change.map(|_| ()).map_err(DbError::Query)
    }

    async fn list(&self, limit: i32) -> Result<Vec<CollectionJobRow>> {
        sqlx::query_as::<_, CollectionJobRow>(
            r#"
            SELECT id, owner, name, full_name, status as "status: _", priority,
                   last_attempt_at, last_completed_at, failure_count, error_message,
                   created_at, updated_at
            FROM collection_jobs
            ORDER BY updated_at DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Query)
    }
}
