use once_cell::sync::Lazy;
use prometheus::{
    register_histogram_vec, register_int_counter_vec, register_int_gauge_vec, HistogramVec,
    IntCounterVec, IntGaugeVec,
};

pub static QUEUE_LENGTH: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "gh_broker_queue_length",
        "Current queue length per budget and priority",
        &["budget", "priority"]
    )
    .expect("queue length metric")
});

pub static SCHEDULED_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "gh_broker_scheduled_total",
        "Total number of scheduled requests per budget and priority",
        &["budget", "priority"]
    )
    .expect("scheduled metric")
});

pub static INFLIGHT: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "gh_broker_inflight",
        "Inflight requests per budget",
        &["budget"]
    )
    .expect("inflight metric")
});

pub static RATE_REMAINING: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "gh_broker_rate_remaining",
        "Rate limit remaining per token and budget",
        &["token", "budget"]
    )
    .expect("rate remaining")
});

pub static RATE_LIMIT: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "gh_broker_rate_limit",
        "Rate limit per token and budget",
        &["token", "budget"]
    )
    .expect("rate limit")
});

pub static SLEEP_SECONDS: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "gh_broker_sleep_seconds_total",
        "Total sleep seconds per budget and reason",
        &["budget", "reason"]
    )
    .expect("sleep seconds")
});

pub static REQUESTS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "gh_broker_requests_total",
        "Requests by budget, token, and status class",
        &["budget", "token", "status"]
    )
    .expect("requests total")
});

pub static RETRIES_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "gh_broker_retries_total",
        "Retries by budget and reason",
        &["budget", "reason"]
    )
    .expect("retries")
});

pub static CACHE_HITS: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "gh_broker_cache_hits_total",
        "Cache hits by budget",
        &["budget"]
    )
    .expect("cache hits")
});

pub static CACHE_MISSES: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "gh_broker_cache_misses_total",
        "Cache misses by budget",
        &["budget"]
    )
    .expect("cache misses")
});

pub static LATENCY: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "gh_broker_latency_seconds",
        "Request latency per budget",
        &["budget"]
    )
    .expect("latency")
});
