use chrono::{DateTime, Utc};
use serde::Serialize;

use db::models::{ActorSpamSummary, IssueRow, RepositoryRow, SpamFlagRow, UserRow};

#[derive(Debug, Serialize)]
pub struct RepoDto {
    pub id: i64,
    pub full_name: String,
    pub is_fork: bool,
    pub created_at: DateTime<Utc>,
    pub pushed_at: Option<DateTime<Utc>>,
}

impl From<RepositoryRow> for RepoDto {
    fn from(row: RepositoryRow) -> Self {
        Self {
            id: row.id,
            full_name: row.full_name,
            is_fork: row.is_fork,
            created_at: row.created_at,
            pushed_at: row.pushed_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct IssueDto {
    pub id: i64,
    pub repo_id: i64,
    pub number: i64,
    pub is_pull_request: bool,
    pub state: String,
    pub title: String,
    pub body: Option<String>,
    pub user_id: Option<i64>,
    pub comments_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub spam_score: Option<f32>,
    pub spam_reasons: Vec<String>,
}

impl IssueDto {
    pub fn from_row(row: IssueRow, spam: Option<f32>, reasons: Vec<String>) -> Self {
        Self {
            id: row.id,
            repo_id: row.repo_id,
            number: row.number,
            is_pull_request: row.is_pull_request,
            state: row.state,
            title: row.title,
            body: row.body,
            user_id: row.user_id,
            comments_count: row.comments_count,
            created_at: row.created_at,
            updated_at: row.updated_at,
            closed_at: row.closed_at,
            spam_score: spam,
            spam_reasons: reasons,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct UserDto {
    pub id: i64,
    pub login: String,
    pub user_type: String,
    pub site_admin: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub followers: Option<i64>,
    pub following: Option<i64>,
    pub public_repos: Option<i64>,
}

impl From<UserRow> for UserDto {
    fn from(row: UserRow) -> Self {
        Self {
            id: row.id,
            login: row.login,
            user_type: row.user_type,
            site_admin: row.site_admin,
            created_at: row.created_at,
            followers: row.followers,
            following: row.following,
            public_repos: row.public_repos,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SpammyUserDto {
    pub login: String,
    pub avg_score: f32,
    pub total_score: f32,
    pub flag_count: i64,
    pub reasons: Vec<String>,
}

impl From<ActorSpamSummary> for SpammyUserDto {
    fn from(summary: ActorSpamSummary) -> Self {
        Self {
            login: summary.login,
            avg_score: summary.avg_score,
            total_score: summary.total_score,
            flag_count: summary.flag_count,
            reasons: summary.reasons,
        }
    }
}

pub fn summarise_flags(flags: &[SpamFlagRow]) -> (Option<f32>, Vec<String>) {
    if flags.is_empty() {
        return (None, Vec::new());
    }
    let mut reasons = Vec::new();
    let mut max_score = None;
    for flag in flags {
        max_score = Some(max_score.map_or(flag.score, |current| current.max(flag.score)));
        for reason in &flag.reasons {
            if !reasons.contains(reason) {
                reasons.push(reason.clone());
            }
        }
    }
    (max_score, reasons)
}
