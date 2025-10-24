use once_cell::sync::Lazy;
use prometheus::{
    register_histogram, register_histogram_vec, register_int_counter, register_int_counter_vec,
    register_int_gauge, register_int_gauge_vec, Histogram, HistogramVec, IntCounter, IntCounterVec,
    IntGauge, IntGaugeVec,
};

pub static RUNS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "collector_runs_total",
        "Total number of collector scheduling cycles attempted"
    )
    .expect("collector runs total")
});

pub static RUN_SUCCESSES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "collector_run_success_total",
        "Collector cycles that completed without repository errors"
    )
    .expect("collector run successes")
});

pub static RUN_FAILURES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "collector_run_failure_total",
        "Collector cycles that completed with at least one repository error"
    )
    .expect("collector run failures")
});

pub static RUN_ERRORS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "collector_run_errors_total",
        "Total number of repository processing errors seen across collector cycles"
    )
    .expect("collector run errors")
});

pub static LAST_RUN_TIMESTAMP: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "collector_last_run_timestamp_seconds",
        "Unix timestamp when the collector cycle last started"
    )
    .expect("collector last run timestamp")
});

pub static LAST_SUCCESS_TIMESTAMP: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "collector_last_success_timestamp_seconds",
        "Unix timestamp when the collector cycle last completed without repository errors"
    )
    .expect("collector last success timestamp")
});

pub static SEED_REPOS: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "collector_seed_repositories",
        "Number of seed repositories loaded for the most recent collector cycle"
    )
    .expect("collector seed repositories gauge")
});

pub static ACTIVE_REPOS: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "collector_active_repositories",
        "Number of repositories currently being processed by the collector"
    )
    .expect("collector active repositories gauge")
});

pub static REPOS_PROCESSED_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "collector_repositories_processed_total",
        "Repositories processed by the collector grouped by outcome",
        &["outcome"]
    )
    .expect("collector repositories processed")
});

pub static ISSUES_PROCESSED_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "collector_issues_processed_total",
        "Total number of issues normalized and upserted by the collector per repository",
        &["repo"]
    )
    .expect("collector issues processed")
});

pub static COMMENTS_PROCESSED_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "collector_comments_processed_total",
        "Total number of comments normalized and upserted by the collector per repository",
        &["repo"]
    )
    .expect("collector comments processed")
});

pub static USERS_FETCHED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "collector_users_fetched_total",
        "Total number of unique users fetched from GitHub during collection"
    )
    .expect("collector users fetched")
});

pub static RUN_DURATION: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "collector_run_duration_seconds",
        "Duration of collector scheduling cycles in seconds",
        vec![1.0, 5.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0]
    )
    .expect("collector run duration histogram")
});

pub static REPO_DURATION: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "collector_repository_duration_seconds",
        "Duration spent processing a repository grouped by outcome",
        &["outcome"],
        vec![0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 120.0]
    )
    .expect("collector repository duration histogram")
});

pub static REPO_JOB_STATUS: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "collector_repository_job_status",
        "Current status of collection jobs per repository (1 if active, 0 if not). Status values: pending, in_progress, completed, failed (transient), error (permanent)",
        &["repo", "status"]
    )
    .expect("collector repository job status")
});

pub static REPO_JOB_FAILURE_COUNT: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "collector_repository_job_failure_count",
        "Number of consecutive failures for a repository collection job",
        &["repo"]
    )
    .expect("collector repository job failure count")
});

pub static REPO_JOB_PRIORITY: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "collector_repository_job_priority",
        "Priority of the repository collection job",
        &["repo"]
    )
    .expect("collector repository job priority")
});

pub static REPO_LAST_ATTEMPT_TIMESTAMP: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "collector_repository_last_attempt_timestamp_seconds",
        "Unix timestamp of the last collection attempt for a repository",
        &["repo"]
    )
    .expect("collector repository last attempt timestamp")
});

pub static REPO_LAST_SUCCESS_TIMESTAMP: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "collector_repository_last_success_timestamp_seconds",
        "Unix timestamp of the last successful collection for a repository",
        &["repo"]
    )
    .expect("collector repository last success timestamp")
});

// Per-fetcher metrics (REST vs GraphQL)
pub static FETCH_REQUESTS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "collector_fetch_requests_total",
        "Total number of fetcher calls grouped by fetcher (rest/graphql), operation, and outcome",
        &["fetcher", "op", "outcome"]
    )
    .expect("collector fetch requests total")
});

pub static FETCH_ITEMS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "collector_fetch_items_total",
        "Number of items returned by fetch operations grouped by fetcher and operation",
        &["fetcher", "op"]
    )
    .expect("collector fetch items total")
});

pub static FETCH_LATENCY_SECONDS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "collector_fetch_latency_seconds",
        "Latency of fetcher calls grouped by fetcher and operation",
        &["fetcher", "op"],
        vec![0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0]
    )
    .expect("collector fetch latency seconds")
});

// Skips: track benign 404s that are intentionally treated as non-fatal
pub static COMMENTS_404_SKIPS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "comments_404_skips_total",
        "Number of comment fetches skipped due to 404 Not Found"
    )
    .expect("comments 404 skips total")
});

// GraphQL rate limit visibility per operation
pub static GQL_RATE_LIMIT_REMAINING: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "gql_rate_limit_remaining",
        "GraphQL remaining points (per operation)",
        &["op"]
    )
    .expect("gql rate remaining")
});

pub static GQL_RATE_LIMIT_LIMIT: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "gql_rate_limit_limit",
        "GraphQL hourly point limit (per operation)",
        &["op"]
    )
    .expect("gql rate limit")
});

pub static GQL_RATE_LIMIT_RESET_TS: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "gql_rate_limit_reset_timestamp_seconds",
        "GraphQL next reset timestamp (per operation)",
        &["op"]
    )
    .expect("gql rate reset ts")
});

pub static GQL_RESOURCE_LIMIT_EVENTS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "gql_resource_limit_events_total",
        "Total GraphQL resource limit events (per operation)",
        &["op"]
    )
    .expect("gql resource limit events")
});

pub static ISSUES_404_SKIPS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "issues_404_skips_total",
        "Number of issue fetches skipped due to 404 Not Found"
    )
    .expect("issues 404 skips total")
});

pub static USERS_404_SKIPS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "users_404_skips_total",
        "Number of user fetches skipped due to 404 Not Found"
    )
    .expect("users 404 skips total")
});

pub struct ActiveRepoGuard;

impl Default for ActiveRepoGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl ActiveRepoGuard {
    pub fn new() -> Self {
        ACTIVE_REPOS.inc();
        Self
    }
}

impl Drop for ActiveRepoGuard {
    fn drop(&mut self) {
        ACTIVE_REPOS.dec();
    }
}
