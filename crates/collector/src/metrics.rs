use once_cell::sync::Lazy;
use prometheus::{
    register_histogram, register_histogram_vec, register_int_counter, register_int_counter_vec,
    register_int_gauge, Histogram, HistogramVec, IntCounter, IntCounterVec, IntGauge,
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

pub static ISSUES_PROCESSED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "collector_issues_processed_total",
        "Total number of issues normalized and upserted by the collector"
    )
    .expect("collector issues processed")
});

pub static COMMENTS_PROCESSED_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "collector_comments_processed_total",
        "Total number of comments normalized and upserted by the collector"
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

pub struct ActiveRepoGuard;

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
