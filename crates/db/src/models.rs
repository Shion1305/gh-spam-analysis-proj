use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct RepositoryRow {
    pub id: i64,
    pub full_name: String,
    pub is_fork: bool,
    pub created_at: DateTime<Utc>,
    pub pushed_at: Option<DateTime<Utc>>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct UserRow {
    pub id: i64,
    pub login: String,
    #[serde(rename = "type")]
    pub user_type: String,
    pub site_admin: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub followers: Option<i64>,
    pub following: Option<i64>,
    pub public_repos: Option<i64>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct IssueRow {
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
    pub dedupe_hash: String,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CommentRow {
    pub id: i64,
    pub issue_id: i64,
    pub user_id: Option<i64>,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub dedupe_hash: String,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SpamFlagRow {
    pub id: i64,
    pub subject_type: String,
    pub subject_id: i64,
    pub score: f32,
    pub reasons: Vec<String>,
    pub version: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CollectorWatermarkRow {
    pub repo_full_name: String,
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct IssueQuery {
    pub repo_full_name: Option<String>,
    pub spam: Option<SpamFilter>,
    pub since: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone)]
pub enum SpamFilter {
    Likely,
    Suspicious,
    All,
}

#[derive(Debug, Clone)]
pub struct SpamFlagUpsert {
    pub subject_type: String,
    pub subject_id: i64,
    pub score: f32,
    pub reasons: Vec<String>,
    pub version: String,
}

#[derive(Debug, Clone)]
pub struct WatermarkUpdate {
    pub repo_full_name: String,
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct TempDb {
    pub name: String,
    pub admin_url: String,
}

impl TempDb {
    pub fn url(&self) -> String {
        format!("{}/{}", self.admin_url, self.name)
    }
}

#[derive(Debug, Clone)]
pub struct ActorSpamSummary {
    pub login: String,
    pub avg_score: f32,
    pub total_score: f32,
    pub flag_count: i64,
    pub reasons: Vec<String>,
}
