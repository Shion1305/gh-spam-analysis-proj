use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use analysis::{score_comment, score_issue, ContributionStats, RuleEngine};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use db::models::{CommentRow, IssueRow, RepositoryRow, SpamFlagUpsert, UserRow, WatermarkUpdate};
use db::Repositories;
use normalizer::models::{
    NormalizedComment, NormalizedIssue, NormalizedRepository, NormalizedUser,
};
use normalizer::{
    normalize_comment, normalize_issue, normalize_repo, normalize_user, CommentPayload,
    IssuePayload, RepoPayload, UserPayload,
};
use serde::Deserialize;
use tokio::time::sleep;
use tracing::{info, instrument, warn};

use crate::client::GithubClient;
use crate::metrics::{self, ActiveRepoGuard};
use common::config::CollectorConfig;

#[derive(Debug, Deserialize)]
pub struct SeedRepo {
    pub owner: String,
    pub name: String,
}

pub struct Collector<C: GithubClient + 'static> {
    config: CollectorConfig,
    client: Arc<C>,
    repos: Arc<dyn Repositories>,
}

struct ProcessContext<'a> {
    rule_version: &'a str,
    user_cache: &'a mut HashSet<String>,
    session_counts: &'a mut HashMap<String, u32>,
    dedupe_counts: &'a mut HashMap<String, u32>,
}

impl<C: GithubClient + 'static> Collector<C> {
    pub fn new(config: CollectorConfig, client: Arc<C>, repos: Arc<dyn Repositories>) -> Self {
        Self {
            config,
            client,
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

        let seeds = match load_seed_repos(&self.config.seed_repos_path)
            .await
            .context("loading seed repositories")
        {
            Ok(seeds) => seeds,
            Err(err) => {
                metrics::RUN_FAILURES_TOTAL.inc();
                metrics::RUN_ERRORS_TOTAL.inc();
                return Err(err);
            }
        };
        metrics::SEED_REPOS.set(seeds.len() as i64);
        info!(count = seeds.len(), "loaded seed repositories");
        let rule_version = RuleEngine::default().version().to_string();
        let mut session_counts = HashMap::new();
        let mut dedupe_counts = HashMap::new();
        let mut repo_errors = 0u64;
        for seed in seeds {
            let repo_started = Instant::now();
            match self
                .process_repo(
                    &seed,
                    &rule_version,
                    &mut session_counts,
                    &mut dedupe_counts,
                )
                .await
            {
                Ok(_) => {
                    metrics::REPOS_PROCESSED_TOTAL
                        .with_label_values(&["success"])
                        .inc();
                    metrics::REPO_DURATION
                        .with_label_values(&["success"])
                        .observe(repo_started.elapsed().as_secs_f64());
                }
                Err(err) => {
                    repo_errors = repo_errors.saturating_add(1);
                    metrics::RUN_ERRORS_TOTAL.inc();
                    metrics::REPOS_PROCESSED_TOTAL
                        .with_label_values(&["error"])
                        .inc();
                    metrics::REPO_DURATION
                        .with_label_values(&["error"])
                        .observe(repo_started.elapsed().as_secs_f64());
                    warn!(
                        owner = %seed.owner,
                        repo = %seed.name,
                        error = ?err,
                        "failed to ingest repository"
                    );
                }
            }
        }
        if repo_errors == 0 {
            metrics::RUN_SUCCESSES_TOTAL.inc();
            metrics::LAST_SUCCESS_TIMESTAMP.set(Utc::now().timestamp());
        } else {
            metrics::RUN_FAILURES_TOTAL.inc();
        }
        Ok(())
    }

    async fn process_repo(
        &self,
        seed: &SeedRepo,
        rule_version: &str,
        session_counts: &mut HashMap<String, u32>,
        dedupe_counts: &mut HashMap<String, u32>,
    ) -> Result<()> {
        let _active_repo = ActiveRepoGuard::new();
        let repo_value = self
            .client
            .get_repo(&seed.owner, &seed.name)
            .await
            .with_context(|| format!("fetching repo {}", seed.name))?;
        let repo_payload: RepoPayload = serde_json::from_value(repo_value.clone())?;
        let normalized_repo = normalize_repo(&repo_payload, repo_value.clone());
        let repo_row = to_repo_row(&normalized_repo);
        self.repos.repos().upsert(repo_row.clone()).await?;
        info!(full_name = %repo_row.full_name, "ingesting repository");

        let watermark = self
            .repos
            .watermarks()
            .get(&repo_row.full_name)
            .await?
            .map(|w| w.last_updated);

        let mut page = 1u32;
        let mut newest_ts: Option<DateTime<Utc>> = watermark;
        let mut user_cache = HashSet::new();
        let mut seen_existing = false;

        loop {
            let issues = self
                .client
                .list_repo_issues(
                    &seed.owner,
                    &seed.name,
                    watermark,
                    page,
                    self.config.page_size,
                )
                .await?;

            if issues.is_empty() {
                break;
            }

            for issue_value in issues {
                let issue_payload: IssuePayload = serde_json::from_value(issue_value.clone())?;
                if let Some(since) = watermark {
                    if issue_payload.updated_at <= since {
                        seen_existing = true;
                        break;
                    }
                }

                let normalized_issue =
                    normalize_issue(&issue_payload, repo_row.id, issue_value.clone());
                let issue_row = to_issue_row(&normalized_issue);
                let (posts_before, user_row) = if let Some(user_ref) = &issue_payload.user {
                    let posts = record_post(session_counts, &user_ref.login);
                    self.ensure_user(&user_ref.login, &mut user_cache).await?;
                    let user_row = self.repos.users().get_by_login(&user_ref.login).await?;
                    (posts, user_row)
                } else {
                    (0, None)
                };
                let dedupe_hits = record_dedupe(dedupe_counts, &normalized_issue.dedupe_hash);

                self.repos.issues().upsert(issue_row.clone()).await?;
                metrics::ISSUES_PROCESSED_TOTAL.inc();
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

                if issue_payload.comments > 0 {
                    let mut ctx = ProcessContext {
                        rule_version,
                        user_cache: &mut user_cache,
                        session_counts,
                        dedupe_counts,
                    };
                    self.process_comments(&issue_row, &seed.owner, &seed.name, &mut ctx)
                        .await?;
                }
            }

            if seen_existing {
                break;
            }

            page += 1;
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

    async fn process_comments(
        &self,
        issue: &IssueRow,
        owner: &str,
        name: &str,
        ctx: &mut ProcessContext<'_>,
    ) -> Result<()> {
        let mut page = 1u32;
        loop {
            let comments = self
                .client
                .list_issue_comments(
                    owner,
                    name,
                    issue.number as u64,
                    page,
                    self.config.page_size,
                )
                .await?;
            if comments.is_empty() {
                break;
            }

            for comment_value in comments {
                let comment_payload: CommentPayload =
                    serde_json::from_value(comment_value.clone())?;
                let normalized_comment =
                    normalize_comment(&comment_payload, issue.id, comment_value.clone());
                let (posts_before, user_row) = if let Some(user_ref) = &comment_payload.user {
                    let posts = record_post(ctx.session_counts, &user_ref.login);
                    self.ensure_user(&user_ref.login, ctx.user_cache).await?;
                    let user_row = self.repos.users().get_by_login(&user_ref.login).await?;
                    (posts, user_row)
                } else {
                    (0, None)
                };
                let dedupe_hits = record_dedupe(ctx.dedupe_counts, &normalized_comment.dedupe_hash);

                let comment_row = to_comment_row(&normalized_comment);
                self.repos.comments().upsert(comment_row.clone()).await?;
                metrics::COMMENTS_PROCESSED_TOTAL.inc();
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
            page += 1;
        }
        Ok(())
    }

    async fn ensure_user(&self, login: &str, cache: &mut HashSet<String>) -> Result<()> {
        if !cache.insert(login.to_string()) {
            return Ok(());
        }
        let user_value = self.client.get_user(login).await?;
        metrics::USERS_FETCHED_TOTAL.inc();
        let user_payload: UserPayload = serde_json::from_value(user_value.clone())?;
        let normalized_user = normalize_user(&user_payload, user_value);
        let user_row = to_user_row(&normalized_user);
        self.repos.users().upsert(user_row).await?;
        Ok(())
    }
}

pub async fn load_seed_repos(path: &str) -> Result<Vec<SeedRepo>> {
    let data = tokio::fs::read_to_string(path).await?;
    let seeds: Vec<SeedRepo> = serde_json::from_str(&data)?;
    Ok(seeds)
}

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
}
