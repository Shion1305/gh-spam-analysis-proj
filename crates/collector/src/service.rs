use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use analysis::{score_comment, score_issue, ContributionStats, RuleEngine};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use db::models::{
    CollectionJobUpdate, CollectionStatus, CommentRow, IssueRow, RepositoryRow, SpamFlagUpsert,
    UserRow, WatermarkUpdate,
};
use db::Repositories;
use http::StatusCode;
use normalizer::models::{
    NormalizedComment, NormalizedIssue, NormalizedRepository, NormalizedUser,
};
use normalizer::payloads::UserRef;
use serde::Deserialize;
use thiserror::Error;
use tokio::time::sleep;
use tracing::{info, instrument, warn};

use crate::client::GithubApiError;
use crate::fetcher::{DataFetcher, UserFetch};
use crate::metrics::{self, ActiveRepoGuard};
use common::config::CollectorConfig;
use gh_broker::HttpStatusError;

#[derive(Debug, Deserialize)]
pub struct SeedRepo {
    pub owner: String,
    pub name: String,
}

pub struct Collector {
    config: CollectorConfig,
    fetcher: Arc<dyn DataFetcher>,
    repos: Arc<dyn Repositories>,
}

struct ProcessContext<'a> {
    rule_version: &'a str,
    user_cache: &'a mut HashSet<String>,
    session_counts: &'a mut HashMap<String, u32>,
    dedupe_counts: &'a mut HashMap<String, u32>,
    repo_full_name: &'a str,
}

#[derive(Debug, Error)]
#[error("seed mismatch: expected '{expected}' but fetched '{actual}'")]
struct SeedMismatchError {
    expected: String,
    actual: String,
}

impl Collector {
    pub fn new(
        config: CollectorConfig,
        fetcher: Arc<dyn DataFetcher>,
        repos: Arc<dyn Repositories>,
    ) -> Self {
        Self {
            config,
            fetcher,
            repos,
        }
    }

    pub async fn run(&self) -> Result<()> {
        loop {
            self.run_once().await?;
            if self.config.run_once {
                break;
            }
            sleep(Duration::from_secs(self.config.interval_secs)).await;
        }
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn run_once(&self) -> Result<()> {
        let run_started = Utc::now();
        metrics::RUNS_TOTAL.inc();
        metrics::LAST_RUN_TIMESTAMP.set(run_started.timestamp());
        let _timer = metrics::RUN_DURATION.start_timer();

        let pending_jobs = match self
            .repos
            .collection_jobs()
            .get_pending(100)
            .await
            .context("loading pending collection jobs")
        {
            Ok(jobs) => jobs,
            Err(err) => {
                metrics::RUN_FAILURES_TOTAL.inc();
                metrics::RUN_ERRORS_TOTAL.inc();
                return Err(err);
            }
        };
        metrics::SEED_REPOS.set(pending_jobs.len() as i64);
        info!(count = pending_jobs.len(), "loaded pending collection jobs");
        let rule_version = RuleEngine::default().version().to_string();
        let repo_errors = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let semaphore = Arc::new(tokio::sync::Semaphore::new(
            self.config.max_concurrent_repos.max(1),
        ));
        let mut join_set = tokio::task::JoinSet::new();

        for job in pending_jobs {
            // Mark job as in progress
            if let Err(err) = self.repos.collection_jobs().mark_in_progress(job.id).await {
                warn!(job_id = job.id, error = ?err, "failed to mark job as in_progress");
                continue;
            }

            // Update metrics for in_progress status
            metrics::REPO_JOB_STATUS
                .with_label_values(&[&job.full_name, "in_progress"])
                .set(1);
            metrics::REPO_JOB_STATUS
                .with_label_values(&[&job.full_name, "pending"])
                .set(0);
            metrics::REPO_JOB_PRIORITY
                .with_label_values(&[&job.full_name])
                .set(job.priority as i64);
            metrics::REPO_LAST_ATTEMPT_TIMESTAMP
                .with_label_values(&[&job.full_name])
                .set(Utc::now().timestamp());
            let semaphore = semaphore.clone();
            let fetcher = self.fetcher.clone();
            let repos = self.repos.clone();
            let rule_version = rule_version.clone();
            let repo_errors = repo_errors.clone();
            let config_clone = self.config.clone();
            join_set.spawn(async move {
                let _permit = semaphore.acquire_owned().await.unwrap();

                let repo_started = Instant::now();
                let seed = SeedRepo {
                    owner: job.owner.clone(),
                    name: job.name.clone(),
                };

                // Per-task counters
                let mut session_counts = HashMap::new();
                let mut dedupe_counts = HashMap::new();
                let c = Collector { config: config_clone, fetcher: fetcher.clone(), repos: repos.clone() };
                let result = c
                    .process_repo(&seed, &rule_version, &mut session_counts, &mut dedupe_counts)
                    .await;

                match result {
                    Ok(_) => {
                        metrics::REPOS_PROCESSED_TOTAL
                            .with_label_values(&["success"])
                            .inc();
                        metrics::REPO_DURATION
                            .with_label_values(&["success"])
                            .observe(repo_started.elapsed().as_secs_f64());
                        if let Err(err) = repos
                            .collection_jobs()
                            .update(CollectionJobUpdate { id: job.id, status: CollectionStatus::Completed, error_message: None })
                            .await
                        {
                            warn!(job_id = job.id, error = ?err, "failed to mark job as completed");
                        } else {
                            metrics::REPO_JOB_STATUS.with_label_values(&[&job.full_name, "completed"]).set(1);
                            metrics::REPO_JOB_STATUS.with_label_values(&[&job.full_name, "pending"]).set(0);
                            metrics::REPO_JOB_STATUS.with_label_values(&[&job.full_name, "in_progress"]).set(0);
                            metrics::REPO_JOB_STATUS.with_label_values(&[&job.full_name, "failed"]).set(0);
                            metrics::REPO_JOB_STATUS.with_label_values(&[&job.full_name, "error"]).set(0);
                            metrics::REPO_JOB_FAILURE_COUNT.with_label_values(&[&job.full_name]).set(0);
                            metrics::REPO_LAST_SUCCESS_TIMESTAMP.with_label_values(&[&job.full_name]).set(Utc::now().timestamp());
                        }
                    }
                    Err(err) => {
                        repo_errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        metrics::RUN_ERRORS_TOTAL.inc();
                        metrics::REPOS_PROCESSED_TOTAL.with_label_values(&["error"]).inc();
                        metrics::REPO_DURATION.with_label_values(&["error"]).observe(repo_started.elapsed().as_secs_f64());

                        let error_details = extract_error_details(&err);
                        let status = if is_permanent_error(&err) { CollectionStatus::Error } else { CollectionStatus::Failed };
                        let error_message = err.to_string();
                        if let Err(update_err) = repos
                            .collection_jobs()
                            .update(CollectionJobUpdate { id: job.id, status: status.clone(), error_message: Some(error_message) })
                            .await
                        {
                            warn!(job_id = job.id, error = ?update_err, "failed to update job status");
                        } else {
                            match status {
                                CollectionStatus::Error => {
                                    metrics::REPO_JOB_STATUS.with_label_values(&[&job.full_name, "error"]).set(1);
                                    metrics::REPO_JOB_STATUS.with_label_values(&[&job.full_name, "pending"]).set(0);
                                    metrics::REPO_JOB_STATUS.with_label_values(&[&job.full_name, "completed"]).set(0);
                                    metrics::REPO_JOB_STATUS.with_label_values(&[&job.full_name, "in_progress"]).set(0);
                                    metrics::REPO_JOB_STATUS.with_label_values(&[&job.full_name, "failed"]).set(0);
                                    metrics::REPO_JOB_FAILURE_COUNT.with_label_values(&[&job.full_name]).set((job.failure_count + 1) as i64);
                                }
                                CollectionStatus::Failed => {
                                    metrics::REPO_JOB_STATUS.with_label_values(&[&job.full_name, "pending"]).set(1);
                                    metrics::REPO_JOB_STATUS.with_label_values(&[&job.full_name, "completed"]).set(0);
                                    metrics::REPO_JOB_STATUS.with_label_values(&[&job.full_name, "in_progress"]).set(0);
                                    metrics::REPO_JOB_STATUS.with_label_values(&[&job.full_name, "failed"]).set(1);
                                    metrics::REPO_JOB_STATUS.with_label_values(&[&job.full_name, "error"]).set(0);
                                    metrics::REPO_JOB_FAILURE_COUNT.with_label_values(&[&job.full_name]).set((job.failure_count + 1) as i64);
                                }
                                _ => {}
                            }
                        }
                        warn!(job_id = job.id, owner = %seed.owner, repo = %seed.name, error = ?err, status_code = error_details.status.map(|s| s.as_u16()), endpoint = error_details.endpoint.as_deref().unwrap_or("-"), context = error_details.context.as_deref().unwrap_or("-"), "job failed");
                    }
                }
            });
        }
        while join_set.join_next().await.is_some() {}

        if repo_errors.load(std::sync::atomic::Ordering::Relaxed) == 0 {
            metrics::RUN_SUCCESSES_TOTAL.inc();
            metrics::LAST_SUCCESS_TIMESTAMP.set(Utc::now().timestamp());
        } else {
            metrics::RUN_FAILURES_TOTAL.inc();
        }
        Ok(())
    }

    #[instrument(
        skip(self, session_counts, dedupe_counts),
        fields(owner = %seed.owner, repo = %seed.name, page_size = self.config.page_size)
    )]
    async fn process_repo(
        &self,
        seed: &SeedRepo,
        rule_version: &str,
        session_counts: &mut HashMap<String, u32>,
        dedupe_counts: &mut HashMap<String, u32>,
    ) -> Result<()> {
        let _active_repo = ActiveRepoGuard::new();
        let repo_full_name = format!("{}/{}", seed.owner, seed.name);
        let repo_snapshot = self
            .fetcher
            .fetch_repo(&seed.owner, &seed.name)
            .await
            .with_context(|| format!("fetching repo {}", seed.name))?;

        // Guard against any mismatch between the requested seed and the fetched repository.
        // This prevents accidentally upserting a different repository row (which can trip
        // unique constraints on repositories.full_name and corrupt subsequent processing).
        let expected_full_name = repo_full_name.to_lowercase();
        let actual_full_name = repo_snapshot.repository.full_name.to_lowercase();
        if expected_full_name != actual_full_name {
            return Err(SeedMismatchError {
                expected: repo_full_name,
                actual: repo_snapshot.repository.full_name.clone(),
            }
            .into());
        }
        let repo_row = to_repo_row(&repo_snapshot.repository);
        self.repos.repos().upsert(repo_row.clone()).await?;
        info!(full_name = %repo_row.full_name, "ingesting repository");

        let watermark = self
            .repos
            .watermarks()
            .get(&repo_row.full_name)
            .await?
            .map(|w| w.last_updated);

        let mut cursor: Option<String> = None;
        let mut newest_ts: Option<DateTime<Utc>> = watermark;
        let mut user_cache = HashSet::new();
        let mut seen_existing = false;

        loop {
            let page = self
                .fetcher
                .fetch_issues(
                    &seed.owner,
                    &seed.name,
                    repo_row.id,
                    watermark,
                    cursor.clone(),
                    self.config.page_size,
                )
                .await?;

            if page.items.is_empty() {
                break;
            }

            for record in page.items {
                let issue = record.issue;
                if let Some(since) = watermark {
                    if issue.updated_at <= since {
                        seen_existing = true;
                        break;
                    }
                }

                let issue_row = to_issue_row(&issue);
                let (posts_before, user_row) = if let Some(user_ref) = &record.author {
                    let posts = record_post(session_counts, &user_ref.login);
                    self.ensure_user(user_ref, &mut user_cache).await?;
                    let user_row = self.repos.users().get_by_login(&user_ref.login).await?;
                    (posts, user_row)
                } else {
                    (0, None)
                };
                let dedupe_hits = record_dedupe(dedupe_counts, &issue.dedupe_hash);

                self.repos.issues().upsert(issue_row.clone()).await?;
                metrics::ISSUES_PROCESSED_TOTAL
                    .with_label_values(&[&repo_full_name])
                    .inc();
                let stats = ContributionStats {
                    posts_last_24h: posts_before,
                    dedupe_hits_last_48h: dedupe_hits,
                };
                let outcome = score_issue(&issue_row, user_row.as_ref(), stats, dedupe_hits);
                if outcome.score > 0.0 {
                    self.repos
                        .spam_flags()
                        .upsert(SpamFlagUpsert {
                            subject_type: "issue".into(),
                            subject_id: issue_row.id,
                            score: outcome.score,
                            reasons: outcome.reasons.clone(),
                            version: rule_version.to_string(),
                        })
                        .await?;
                }
                newest_ts = Some(match newest_ts {
                    Some(existing) if existing > issue_row.updated_at => existing,
                    _ => issue_row.updated_at,
                });

                if issue_row.comments_count > 0 {
                    let mut ctx = ProcessContext {
                        rule_version,
                        user_cache: &mut user_cache,
                        session_counts,
                        dedupe_counts,
                        repo_full_name: &repo_full_name,
                    };
                    self.process_comments(&issue_row, &seed.owner, &seed.name, &mut ctx)
                        .await?;
                }
            }

            if seen_existing {
                break;
            }

            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }

        if let Some(ts) = newest_ts {
            self.repos
                .watermarks()
                .set(WatermarkUpdate {
                    repo_full_name: repo_row.full_name.clone(),
                    last_updated: ts,
                })
                .await?;
        }

        Ok(())
    }

    #[instrument(
        skip(self, ctx),
        fields(
            repo = ctx.repo_full_name,
            issue_number = issue.number,
            page_size = self.config.page_size
        )
    )]
    async fn process_comments(
        &self,
        issue: &IssueRow,
        owner: &str,
        name: &str,
        ctx: &mut ProcessContext<'_>,
    ) -> Result<()> {
        let mut cursor: Option<String> = None;
        loop {
            let page = match self
                .fetcher
                .fetch_issue_comments(
                    owner,
                    name,
                    issue.number,
                    issue.id,
                    cursor.clone(),
                    self.config.page_size,
                )
                .await
            {
                Ok(page) => page,
                Err(err) => {
                    let details = extract_error_details(&err);
                    if matches!(details.status, Some(StatusCode::NOT_FOUND)) {
                        warn!(
                            repo = ctx.repo_full_name,
                            issue_number = issue.number,
                            status_code = details.status.map(|s| s.as_u16()),
                            "issue not found while listing comments"
                        );
                        if issue.found {
                            let mut missing_issue = issue.clone();
                            missing_issue.found = false;
                            self.repos.issues().upsert(missing_issue).await?;
                        }
                        return Ok(());
                    }
                    return Err(err);
                }
            };

            if page.items.is_empty() {
                break;
            }

            for record in page.items {
                let comment = record.comment;
                let (posts_before, user_row) = if let Some(user_ref) = &record.author {
                    let posts = record_post(ctx.session_counts, &user_ref.login);
                    self.ensure_user(user_ref, ctx.user_cache).await?;
                    let user_row = self.repos.users().get_by_login(&user_ref.login).await?;
                    (posts, user_row)
                } else {
                    (0, None)
                };
                let dedupe_hits = record_dedupe(ctx.dedupe_counts, &comment.dedupe_hash);

                let comment_row = to_comment_row(&comment);
                self.repos.comments().upsert(comment_row.clone()).await?;
                metrics::COMMENTS_PROCESSED_TOTAL
                    .with_label_values(&[ctx.repo_full_name])
                    .inc();
                let stats = ContributionStats {
                    posts_last_24h: posts_before,
                    dedupe_hits_last_48h: dedupe_hits,
                };
                let outcome = score_comment(&comment_row, user_row.as_ref(), stats, dedupe_hits);
                if outcome.score > 0.0 {
                    self.repos
                        .spam_flags()
                        .upsert(SpamFlagUpsert {
                            subject_type: "comment".into(),
                            subject_id: comment_row.id,
                            score: outcome.score,
                            reasons: outcome.reasons.clone(),
                            version: ctx.rule_version.to_string(),
                        })
                        .await?;
                }
            }

            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        Ok(())
    }

    async fn ensure_user(&self, user_ref: &UserRef, cache: &mut HashSet<String>) -> Result<()> {
        if !cache.insert(user_ref.login.clone()) {
            return Ok(());
        }
        match self.fetcher.fetch_user(user_ref).await? {
            UserFetch::Found(normalized_user) => {
                metrics::USERS_FETCHED_TOTAL.inc();
                let mut user_row = to_user_row(&normalized_user);
                user_row.found = true;
                self.repos.users().upsert(user_row).await?;
                Ok(())
            }
            UserFetch::Missing(missing) => {
                warn!(
                    login = %missing.login,
                    user_id = missing.id,
                    status_code = missing.status.map(|s| s.as_u16()),
                    "user not found - marking as missing"
                );
                let placeholder = UserRow {
                    id: missing.id,
                    login: missing.login.clone(),
                    user_type: "unknown".to_string(),
                    site_admin: false,
                    created_at: None,
                    followers: None,
                    following: None,
                    public_repos: None,
                    raw: serde_json::json!({"not_found": true}),
                    found: false,
                };
                self.repos.users().upsert(placeholder).await?;
                Ok(())
            }
        }
    }
}

// Historical helper removed: seeding is now done via collection_jobs and the API.

fn to_repo_row(normalized: &NormalizedRepository) -> RepositoryRow {
    RepositoryRow {
        id: normalized.id,
        full_name: normalized.full_name.clone(),
        is_fork: normalized.is_fork,
        created_at: normalized.created_at,
        pushed_at: normalized.pushed_at,
        raw: normalized.raw.clone(),
    }
}

fn to_user_row(normalized: &NormalizedUser) -> UserRow {
    UserRow {
        id: normalized.id,
        login: normalized.login.clone(),
        user_type: normalized.user_type.clone(),
        site_admin: normalized.site_admin,
        created_at: normalized.created_at,
        followers: normalized.followers,
        following: normalized.following,
        public_repos: normalized.public_repos,
        raw: normalized.raw.clone(),
        found: true,
    }
}

fn to_issue_row(normalized: &NormalizedIssue) -> IssueRow {
    IssueRow {
        id: normalized.id,
        repo_id: normalized.repo_id,
        number: normalized.number,
        is_pull_request: normalized.is_pull_request,
        state: normalized.state.clone(),
        title: normalized.title.clone(),
        body: normalized.body.clone(),
        user_id: normalized.user_id,
        comments_count: normalized.comments_count,
        created_at: normalized.created_at,
        updated_at: normalized.updated_at,
        closed_at: normalized.closed_at,
        dedupe_hash: normalized.dedupe_hash.clone(),
        raw: normalized.raw.clone(),
        found: true,
    }
}

fn to_comment_row(normalized: &NormalizedComment) -> CommentRow {
    CommentRow {
        id: normalized.id,
        issue_id: normalized.issue_id,
        user_id: normalized.user_id,
        body: normalized.body.clone(),
        created_at: normalized.created_at,
        updated_at: normalized.updated_at,
        dedupe_hash: normalized.dedupe_hash.clone(),
        raw: normalized.raw.clone(),
        found: true,
    }
}

fn record_post(counts: &mut HashMap<String, u32>, login: &str) -> u32 {
    let entry = counts.entry(login.to_string()).or_insert(0);
    let current = *entry;
    *entry = current.saturating_add(1);
    current
}

fn record_dedupe(counts: &mut HashMap<String, u32>, hash: &str) -> u32 {
    let entry = counts.entry(hash.to_string()).or_insert(0);
    let current = *entry;
    *entry = current.saturating_add(1);
    current
}

/// Determines if an error is permanent (will not retry) or transient (will retry)
///
/// Permanent errors include:
/// - 404 Not Found - repository doesn't exist
/// - 403 Forbidden - access denied, private repo, or suspended account
/// - 451 Unavailable For Legal Reasons - DMCA takedown, etc.
/// - 410 Gone - resource permanently deleted
///
/// Transient errors include:
/// - 5xx Server errors - GitHub API issues
/// - 429 Rate limit - will retry when rate limit resets
/// - Network errors - timeouts, connection issues
/// - Other errors - parsing issues, etc.
fn is_permanent_error(err: &anyhow::Error) -> bool {
    // Explicitly treat seed mismatch as permanent to avoid useless retries
    if err.downcast_ref::<SeedMismatchError>().is_some() {
        return true;
    }
    if let Some(status) = status_from_error(err) {
        match status {
            StatusCode::NOT_FOUND => {
                if let Some(endpoint) = endpoint_from_error(err) {
                    let segments: Vec<&str> = endpoint
                        .trim_matches('/')
                        .split('/')
                        .filter(|s| !s.is_empty())
                        .collect();
                    if segments.len() == 3 && segments[0] == "repos" {
                        return true;
                    }
                }
            }
            StatusCode::FORBIDDEN
            | StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS
            | StatusCode::GONE => return true,
            _ => {}
        }
    }

    // Fallback to string matching when error type is unknown
    let error_msg = err.to_string();
    if error_msg.contains("404") {
        let is_repo_fetch = error_msg.contains("fetching repo")
            || (error_msg.contains("github api error: 404")
                && error_msg.contains("repos/")
                && !error_msg.contains("issues")
                && !error_msg.contains("comments"));
        return is_repo_fetch;
    }
    error_msg.contains("403") || error_msg.contains("451") || error_msg.contains("410")
}

#[derive(Default)]
struct ErrorDetails {
    status: Option<StatusCode>,
    endpoint: Option<String>,
    context: Option<String>,
}

fn extract_error_details(err: &anyhow::Error) -> ErrorDetails {
    let status = status_from_error(err);
    let endpoint = endpoint_from_error(err);
    let context = err
        .chain()
        .map(|cause| cause.to_string())
        .find(|value| !value.trim().is_empty())
        .map(|value| truncate_context(&value))
        .unwrap_or_else(|| truncate_context(&err.to_string()));

    ErrorDetails {
        status,
        endpoint,
        context: Some(context),
    }
}

fn truncate_context(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let mut truncated: String = value.chars().take(256).collect();
    if truncated.len() < value.len() {
        truncated.push('…');
    }
    truncated
}

fn status_from_error(err: &anyhow::Error) -> Option<StatusCode> {
    for cause in err.chain() {
        if let Some(api_err) = cause.downcast_ref::<GithubApiError>() {
            return Some(api_err.status_code());
        }
        if let Some(http_err) = cause.downcast_ref::<HttpStatusError>() {
            return Some(http_err.status);
        }
    }

    err.chain()
        .filter_map(|cause| parse_status_from_message(&cause.to_string()))
        .next()
        .or_else(|| parse_status_from_message(&err.to_string()))
}

fn endpoint_from_error(err: &anyhow::Error) -> Option<String> {
    for cause in err.chain() {
        if let Some(api_err) = cause.downcast_ref::<GithubApiError>() {
            return Some(api_err.endpoint().to_string());
        }
        if let Some(endpoint) = parse_endpoint_from_message(&cause.to_string()) {
            return Some(endpoint);
        }
    }

    parse_endpoint_from_message(&err.to_string())
}

fn parse_status_from_message(message: &str) -> Option<StatusCode> {
    for part in message.split(|ch: char| !ch.is_ascii_digit()) {
        if part.len() == 3 {
            if let Ok(code) = part.parse::<u16>() {
                if let Ok(status) = StatusCode::from_u16(code) {
                    return Some(status);
                }
            }
        }
    }
    None
}

fn parse_endpoint_from_message(message: &str) -> Option<String> {
    if let Some(idx) = message.rfind(" for ") {
        let endpoint = message[idx + 5..].trim_matches(|ch: char| ch == '"' || ch.is_whitespace());
        if !endpoint.is_empty() {
            return Some(endpoint.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_post_returns_previous_count() {
        let mut counts = HashMap::new();
        assert_eq!(record_post(&mut counts, "alice"), 0);
        assert_eq!(record_post(&mut counts, "alice"), 1);
    }

    #[test]
    fn record_dedupe_returns_previous_count() {
        let mut counts = HashMap::new();
        assert_eq!(record_dedupe(&mut counts, "hash"), 0);
        assert_eq!(record_dedupe(&mut counts, "hash"), 1);
    }

    #[test]
    fn repo_not_found_is_permanent() {
        let err = anyhow::Error::new(GithubApiError::status(
            StatusCode::NOT_FOUND,
            "repos/foo/bar",
        ));
        assert!(is_permanent_error(&err));
    }

    #[test]
    fn issue_not_found_is_transient() {
        let err = anyhow::Error::new(GithubApiError::status(
            StatusCode::NOT_FOUND,
            "repos/foo/bar/issues",
        ));
        assert!(!is_permanent_error(&err));
    }

    #[test]
    fn user_not_found_is_transient() {
        let err = anyhow::Error::new(GithubApiError::status(
            StatusCode::NOT_FOUND,
            "users/someone",
        ));
        assert!(!is_permanent_error(&err));
    }

    #[test]
    fn comment_not_found_is_transient() {
        let err = anyhow::Error::new(GithubApiError::status(
            StatusCode::NOT_FOUND,
            "repos/foo/bar/issues/comments",
        ));
        assert!(!is_permanent_error(&err));
    }

    #[test]
    fn extract_error_details_captures_status_from_http_error() {
        let err = anyhow::Error::new(HttpStatusError::new(StatusCode::NOT_FOUND));
        let details = extract_error_details(&err);
        assert_eq!(details.status, Some(StatusCode::NOT_FOUND));
        assert_eq!(details.endpoint, None);
    }

    #[test]
    fn extract_error_details_captures_endpoint_from_github_error() {
        let err = anyhow::Error::new(GithubApiError::status(
            StatusCode::NOT_FOUND,
            "users/example",
        ));
        let details = extract_error_details(&err);
        assert_eq!(details.status, Some(StatusCode::NOT_FOUND));
        assert_eq!(details.endpoint.as_deref(), Some("users/example"));
    }

    #[test]
    fn status_from_error_handles_plain_text_message() {
        let err = anyhow::anyhow!("unexpected status 404 Not Found");
        assert_eq!(status_from_error(&err), Some(StatusCode::NOT_FOUND));
    }

    #[test]
    fn extract_error_details_from_plain_message_recovers_endpoint() {
        let err = anyhow::anyhow!("unexpected status 404 Not Found for users/example");
        let details = extract_error_details(&err);
        assert_eq!(details.status, Some(StatusCode::NOT_FOUND));
        assert_eq!(details.endpoint.as_deref(), Some("users/example"));
    }
}
