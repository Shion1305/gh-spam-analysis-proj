use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::errors::Result;
use crate::models::{
    ActorSpamSummary, CollectorWatermarkRow, CommentRow, IssueQuery, IssueRow, RepositoryRow,
    SpamFlagRow, SpamFlagUpsert, UserRow, WatermarkUpdate,
};

#[async_trait]
pub trait RepoRepository: Send + Sync {
    async fn upsert(&self, repo: RepositoryRow) -> Result<()>;
    async fn get_by_full_name(&self, full_name: &str) -> Result<Option<RepositoryRow>>;
    async fn list(&self, limit: i64) -> Result<Vec<RepositoryRow>>;
}

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn upsert(&self, user: UserRow) -> Result<()>;
    async fn get_by_id(&self, id: i64) -> Result<Option<UserRow>>;
    async fn get_by_login(&self, login: &str) -> Result<Option<UserRow>>;
}

#[async_trait]
pub trait IssueRepository: Send + Sync {
    async fn upsert(&self, issue: IssueRow) -> Result<()>;
    async fn query(&self, query: IssueQuery) -> Result<Vec<IssueRow>>;
    async fn list_by_repo(
        &self,
        repo_id: i64,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<IssueRow>>;
}

#[async_trait]
pub trait CommentRepository: Send + Sync {
    async fn upsert(&self, comment: CommentRow) -> Result<()>;
    async fn list_by_issue(&self, issue_id: i64) -> Result<Vec<CommentRow>>;
}

#[async_trait]
pub trait WatermarkRepository: Send + Sync {
    async fn get(&self, repo_full_name: &str) -> Result<Option<CollectorWatermarkRow>>;
    async fn set(&self, watermark: WatermarkUpdate) -> Result<()>;
}

#[async_trait]
pub trait SpamFlagsRepository: Send + Sync {
    async fn upsert(&self, flag: SpamFlagUpsert) -> Result<()>;
    async fn list_for_subject(
        &self,
        subject_type: &str,
        subject_id: i64,
    ) -> Result<Vec<SpamFlagRow>>;
    async fn top_spammy_users(
        &self,
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<ActorSpamSummary>>;
}

pub trait Repositories: Send + Sync {
    fn repos(&self) -> &dyn RepoRepository;
    fn users(&self) -> &dyn UserRepository;
    fn issues(&self) -> &dyn IssueRepository;
    fn comments(&self) -> &dyn CommentRepository;
    fn watermarks(&self) -> &dyn WatermarkRepository;
    fn spam_flags(&self) -> &dyn SpamFlagsRepository;
}
