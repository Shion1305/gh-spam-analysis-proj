use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct RepoPayload {
    pub id: i64,
    pub full_name: String,
    #[serde(default)]
    pub fork: bool,
    pub created_at: DateTime<Utc>,
    pub pushed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserPayload {
    pub id: i64,
    pub login: String,
    #[serde(rename = "type")]
    pub user_type: String,
    #[serde(default)]
    pub site_admin: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub followers: Option<i64>,
    pub following: Option<i64>,
    pub public_repos: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IssuePayload {
    pub id: i64,
    pub number: i64,
    pub pull_request: Option<serde_json::Value>,
    pub state: String,
    pub title: String,
    pub body: Option<String>,
    pub user: Option<UserRef>,
    pub comments: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommentPayload {
    pub id: i64,
    pub user: Option<UserRef>,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserRef {
    pub id: i64,
    pub login: String,
}
