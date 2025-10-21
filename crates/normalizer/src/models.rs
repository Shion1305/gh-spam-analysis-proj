use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedRepository {
    pub id: i64,
    pub full_name: String,
    pub is_fork: bool,
    pub created_at: DateTime<Utc>,
    pub pushed_at: Option<DateTime<Utc>>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedUser {
    pub id: i64,
    pub login: String,
    pub user_type: String,
    pub site_admin: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub followers: Option<i64>,
    pub following: Option<i64>,
    pub public_repos: Option<i64>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedIssue {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedComment {
    pub id: i64,
    pub issue_id: i64,
    pub user_id: Option<i64>,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub dedupe_hash: String,
    pub raw: serde_json::Value,
}
