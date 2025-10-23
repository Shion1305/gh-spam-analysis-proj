use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, NaiveDate, Utc};
use db::models::{CollectionJobCreate, IssueQuery, SpamFilter};
use db::Repositories;
use once_cell::sync::Lazy;
use prometheus::{register_int_gauge_vec, Encoder, IntGaugeVec};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use tracing::instrument;

use crate::dto::{summarise_flags, IssueDto, RepoDto, SpammyUserDto, UserDto};
use crate::error::{ApiError, ApiResult};

#[derive(Clone)]
pub struct ApiState {
    pub repositories: Arc<dyn Repositories>,
    pub metrics_path: &'static str,
    pub pool: Arc<PgPool>,
}

pub fn build_router(state: Arc<ApiState>) -> Router {
    let metrics_path: &'static str = state.metrics_path;
    Router::new()
        .route("/healthz", get(healthz))
        .route("/repos", get(list_repos).post(register_repo))
        .route("/collection-jobs", get(list_collection_jobs))
        .route("/issues", get(list_issues))
        .route("/actors/:login", get(get_actor))
        .route("/top/spammy-users", get(top_spammy_users))
        .route(metrics_path, get(metrics))
        .with_state(state)
}

static ISSUES_BY_REPO: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "db_issues_total_by_repo",
        "Number of issues stored per repository",
        &["repo"]
    )
    .expect("issues_by_repo gauge")
});

static COMMENTS_BY_REPO: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "db_comments_total_by_repo",
        "Number of comments stored per repository",
        &["repo"]
    )
    .expect("comments_by_repo gauge")
});

static USERS_BY_REPO: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "db_users_total_by_repo",
        "Number of distinct users participating (issues + comments) per repository",
        &["repo"]
    )
    .expect("users_by_repo gauge")
});

async fn refresh_repo_entity_counts(pool: &PgPool) -> Result<(), String> {
    // Issues per repo
    let issues = sqlx::query!(
        r#"
        SELECT r.full_name as repo, COUNT(*)::BIGINT as count
        FROM issues i
        JOIN repositories r ON r.id = i.repo_id
        GROUP BY r.full_name
        "#
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    for row in &issues {
        let repo: &str = &row.repo;
        let cnt = row.count.unwrap_or(0);
        ISSUES_BY_REPO.with_label_values(&[repo]).set(cnt);
    }

    // Comments per repo
    let comments = sqlx::query!(
        r#"
        SELECT r.full_name as repo, COUNT(*)::BIGINT as count
        FROM comments c
        JOIN issues i ON i.id = c.issue_id
        JOIN repositories r ON r.id = i.repo_id
        GROUP BY r.full_name
        "#
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    for row in &comments {
        let repo: &str = &row.repo;
        let cnt = row.count.unwrap_or(0);
        COMMENTS_BY_REPO.with_label_values(&[repo]).set(cnt);
    }

    // Distinct users (issue authors + comment authors) per repo
    let users = sqlx::query!(
        r#"
        WITH iu AS (
            SELECT r.full_name as repo, i.user_id as uid
            FROM issues i
            JOIN repositories r ON r.id = i.repo_id
            WHERE i.user_id IS NOT NULL
        ),
        cu AS (
            SELECT r.full_name as repo, c.user_id as uid
            FROM comments c
            JOIN issues i ON i.id = c.issue_id
            JOIN repositories r ON r.id = i.repo_id
            WHERE c.user_id IS NOT NULL
        ),
        allu AS (
            SELECT repo, uid FROM iu
            UNION ALL
            SELECT repo, uid FROM cu
        )
        SELECT repo, COUNT(DISTINCT uid)::BIGINT AS count
        FROM allu
        GROUP BY repo
        "#
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    for row in &users {
        let repo = row.repo.as_deref().unwrap_or("");
        let cnt = row.count.unwrap_or(0);
        USERS_BY_REPO.with_label_values(&[repo]).set(cnt);
    }

    Ok(())
}

async fn healthz() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

#[derive(Debug, Deserialize)]
struct RepoQuery {
    limit: Option<i64>,
}

#[instrument(skip(state))]
async fn list_repos(
    State(state): State<Arc<ApiState>>,
    Query(query): Query<RepoQuery>,
) -> ApiResult<Json<Vec<RepoDto>>> {
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let rows = state.repositories.repos().list(limit).await?;
    let dto = rows.into_iter().map(RepoDto::from).collect();
    Ok(Json(dto))
}

#[derive(Debug, Deserialize)]
struct IssuesQuery {
    repo: Option<String>,
    spam: Option<String>,
    since: Option<String>,
    limit: Option<i64>,
}

#[instrument(skip(state))]
async fn list_issues(
    State(state): State<Arc<ApiState>>,
    Query(query): Query<IssuesQuery>,
) -> ApiResult<Json<Vec<IssueDto>>> {
    let issue_query = IssueQuery {
        repo_full_name: query.repo,
        limit: query.limit.map(|l| l.clamp(1, 200)),
        spam: query.spam.as_deref().map(parse_spam_filter).transpose()?,
        since: match query.since {
            Some(ref value) => Some(parse_since(value)?),
            None => None,
        },
    };

    let rows = state.repositories.issues().query(issue_query).await?;
    let mut issues = Vec::with_capacity(rows.len());
    for issue in rows {
        let flags = state
            .repositories
            .spam_flags()
            .list_for_subject("issue", issue.id)
            .await?;
        let (score, reasons) = summarise_flags(&flags);
        issues.push(IssueDto::from_row(issue, score, reasons));
    }
    Ok(Json(issues))
}

#[instrument(skip(state))]
async fn get_actor(
    State(state): State<Arc<ApiState>>,
    Path(login): Path<String>,
) -> ApiResult<Json<UserDto>> {
    let user = state
        .repositories
        .users()
        .get_by_login(&login)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("user {} not found", login)))?;
    Ok(Json(UserDto::from(user)))
}

#[derive(Debug, Deserialize)]
struct SpammyUsersQuery {
    since: Option<String>,
    limit: Option<i64>,
}

#[instrument(skip(state))]
async fn top_spammy_users(
    State(state): State<Arc<ApiState>>,
    Query(query): Query<SpammyUsersQuery>,
) -> ApiResult<Json<Vec<SpammyUserDto>>> {
    let since = match query.since {
        Some(ref value) => Some(parse_since(value)?),
        None => None,
    };
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let rows = state
        .repositories
        .spam_flags()
        .top_spammy_users(since, limit)
        .await?;
    Ok(Json(rows.into_iter().map(SpammyUserDto::from).collect()))
}

#[instrument(skip(state))]
async fn metrics(State(state): State<Arc<ApiState>>) -> ApiResult<impl IntoResponse> {
    // Best-effort: refresh counts before scraping metrics
    if let Err(err) = refresh_repo_entity_counts(&state.pool).await {
        tracing::warn!(error = %err, "failed to refresh repo entity counts");
    }
    let encoder = prometheus::TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    let content_type = encoder.format_type().to_string();
    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok((
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, content_type)],
        buffer,
    ))
}

fn parse_spam_filter(value: &str) -> ApiResult<SpamFilter> {
    match value.to_ascii_lowercase().as_str() {
        "likely" => Ok(SpamFilter::Likely),
        "suspicious" => Ok(SpamFilter::Suspicious),
        "all" => Ok(SpamFilter::All),
        other => Err(ApiError::bad_request(format!(
            "invalid spam filter: {}",
            other
        ))),
    }
}

fn parse_since(value: &str) -> ApiResult<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        if let Some(dt) = date.and_hms_opt(0, 0, 0) {
            return Ok(dt.and_utc());
        }
    }
    Err(ApiError::bad_request("invalid since parameter"))
}

#[derive(Debug, Deserialize)]
struct RegisterRepoRequest {
    owner: String,
    name: String,
    #[serde(default)]
    priority: i32,
}

#[derive(Debug, Serialize)]
struct CollectionJobResponse {
    id: i64,
    owner: String,
    name: String,
    full_name: String,
    status: String,
    priority: i32,
    failure_count: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[instrument(skip(state))]
async fn register_repo(
    State(state): State<Arc<ApiState>>,
    Json(request): Json<RegisterRepoRequest>,
) -> ApiResult<Json<CollectionJobResponse>> {
    let job = state
        .repositories
        .collection_jobs()
        .create(CollectionJobCreate {
            owner: request.owner,
            name: request.name,
            priority: request.priority,
        })
        .await?;

    Ok(Json(CollectionJobResponse {
        id: job.id,
        owner: job.owner,
        name: job.name,
        full_name: job.full_name,
        status: format!("{:?}", job.status),
        priority: job.priority,
        failure_count: job.failure_count,
        created_at: job.created_at,
        updated_at: job.updated_at,
    }))
}

#[derive(Debug, Deserialize)]
struct CollectionJobsQuery {
    limit: Option<i32>,
}

#[instrument(skip(state))]
async fn list_collection_jobs(
    State(state): State<Arc<ApiState>>,
    Query(query): Query<CollectionJobsQuery>,
) -> ApiResult<Json<Vec<CollectionJobResponse>>> {
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let jobs = state.repositories.collection_jobs().list(limit).await?;

    let response = jobs
        .into_iter()
        .map(|job| CollectionJobResponse {
            id: job.id,
            owner: job.owner,
            name: job.name,
            full_name: job.full_name,
            status: format!("{:?}", job.status),
            priority: job.priority,
            failure_count: job.failure_count,
            created_at: job.created_at,
            updated_at: job.updated_at,
        })
        .collect();

    Ok(Json(response))
}
