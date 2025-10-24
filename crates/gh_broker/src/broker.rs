use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use futures::FutureExt;
use http::{header, HeaderValue, Request, Response, StatusCode};
use tokio::sync::{mpsc, oneshot, Mutex, Semaphore};
use tokio::time::sleep;
use tracing::warn;

use crate::backoff::exponential_jitter_backoff;
use crate::cache::{CachedResponse, ResponseCache};
use crate::error::HttpStatusError;
use crate::metrics;
use crate::model::{parse_rate_limit, parse_retry_after, Budget, GithubRequest};
use crate::token::{GithubToken, TokenPool, TokenSelection};

#[async_trait]
pub trait HttpExec: Send + Sync {
    async fn execute(&self, req: Request<Vec<u8>>) -> Result<Response<Vec<u8>>>;
}

pub struct ReqwestExecutor {
    client: reqwest::Client,
}

impl Default for ReqwestExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl ReqwestExecutor {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent("github-spam-lab")
            .build()
            .expect("reqwest client");
        Self { client }
    }
}

#[async_trait]
impl HttpExec for ReqwestExecutor {
    async fn execute(&self, req: Request<Vec<u8>>) -> Result<Response<Vec<u8>>> {
        let (parts, body) = req.into_parts();
        let mut builder = self.client.request(parts.method, parts.uri.to_string());
        builder = builder.headers(parts.headers);
        let resp = builder.body(body).send().await?;
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = resp.bytes().await?;
        let mut builder = Response::builder().status(status);
        *builder.headers_mut().unwrap() = headers;
        Ok(builder.body(bytes.to_vec())?)
    }
}

pub trait GithubBroker: Send + Sync {
    fn enqueue(
        &self,
        request: Request<Vec<u8>>,
        priority: crate::model::Priority,
    ) -> futures::future::BoxFuture<'static, Result<Response<Vec<u8>>>>;
}

struct WorkItem {
    request: GithubRequest,
    key: String,
    cached: Option<CachedResponse>,
}

type QueueSenders = HashMap<(Budget, crate::model::Priority), mpsc::Sender<WorkItem>>;
type QueueReceivers = Vec<(
    Budget,
    (
        mpsc::Receiver<WorkItem>,
        mpsc::Receiver<WorkItem>,
        mpsc::Receiver<WorkItem>,
    ),
)>;
type ReceiversMap = HashMap<
    Budget,
    (
        Option<mpsc::Receiver<WorkItem>>,
        Option<mpsc::Receiver<WorkItem>>,
        Option<mpsc::Receiver<WorkItem>>,
    ),
>;

#[derive(Clone)]
pub struct GithubBrokerBuilder {
    tokens: Vec<GithubToken>,
    http_exec: Option<Arc<dyn HttpExec>>,
    queue_bounds: HashMap<(Budget, crate::model::Priority), usize>,
    weights: HashMap<Budget, [u32; 3]>,
    max_inflight: usize,
    per_repo_inflight: usize,
    cache_capacity: usize,
    cache_ttl: Duration,
    backoff_base: Duration,
    backoff_max: Duration,
    jitter_frac: f32,
}

impl GithubBrokerBuilder {
    pub fn new(tokens: Vec<GithubToken>) -> Self {
        let mut queue_bounds = HashMap::new();
        queue_bounds.insert((Budget::Core, crate::model::Priority::Critical), 2048);
        queue_bounds.insert((Budget::Core, crate::model::Priority::Normal), 4096);
        queue_bounds.insert((Budget::Core, crate::model::Priority::Backfill), 4096);
        queue_bounds.insert((Budget::Search, crate::model::Priority::Normal), 512);
        queue_bounds.insert((Budget::Graphql, crate::model::Priority::Normal), 1024);

        let mut weights = HashMap::new();
        weights.insert(Budget::Core, [4, 2, 1]);
        weights.insert(Budget::Search, [2, 1, 0]);
        weights.insert(Budget::Graphql, [3, 2, 1]);

        Self {
            tokens,
            http_exec: None,
            queue_bounds,
            weights,
            max_inflight: 32,
            per_repo_inflight: 2,
            cache_capacity: 5000,
            cache_ttl: Duration::from_secs(600),
            backoff_base: Duration::from_millis(500),
            backoff_max: Duration::from_millis(60_000),
            jitter_frac: 0.2,
        }
    }

    pub fn http_exec(mut self, exec: Arc<dyn HttpExec>) -> Self {
        self.http_exec = Some(exec);
        self
    }

    pub fn max_inflight(mut self, max: usize) -> Self {
        self.max_inflight = max;
        self
    }

    pub fn per_repo_inflight(mut self, max: usize) -> Self {
        self.per_repo_inflight = max;
        self
    }

    pub fn cache(mut self, capacity: usize, ttl: Duration) -> Self {
        self.cache_capacity = capacity;
        self.cache_ttl = ttl;
        self
    }

    pub fn queue_bounds(
        mut self,
        bounds: HashMap<(Budget, crate::model::Priority), usize>,
    ) -> Self {
        self.queue_bounds = bounds;
        self
    }

    pub fn weights(mut self, weights: HashMap<Budget, [u32; 3]>) -> Self {
        self.weights = weights;
        self
    }

    pub fn backoff(mut self, base: Duration, max: Duration, jitter: f32) -> Self {
        self.backoff_base = base;
        self.backoff_max = max;
        self.jitter_frac = jitter;
        self
    }

    pub fn build(self) -> Arc<dyn GithubBroker> {
        let exec = self
            .http_exec
            .unwrap_or_else(|| Arc::new(ReqwestExecutor::new()));

        let token_pool = TokenPool::new(self.tokens.clone());
        let cache = ResponseCache::new(self.cache_capacity, self.cache_ttl);

        let (senders, receivers) = Self::build_queues(&self.queue_bounds);

        let inner = Arc::new(Inner {
            http_exec: exec,
            token_pool,
            cache,
            pending: Mutex::new(HashMap::new()),
            inflight: Arc::new(Semaphore::new(self.max_inflight)),
            per_repo_limit: self.per_repo_inflight,
            per_repo: Mutex::new(HashMap::new()),
            backoff_base: self.backoff_base,
            backoff_max: self.backoff_max,
            jitter: self.jitter_frac,
        });

        for (budget, (rx_crit, rx_norm, rx_back)) in receivers {
            let inner = inner.clone();
            let weights = self.weights.get(&budget).cloned().unwrap_or([1, 1, 1]);
            tokio::spawn(async move {
                run_budget(inner, budget, rx_crit, rx_norm, rx_back, weights).await;
            });
        }

        // Background metrics refresh loop: propagate per-token and aggregated
        // budget metrics even when no work is being processed.
        {
            let inner = inner.clone();
            tokio::spawn(async move {
                use tokio::time::{sleep, Duration};
                loop {
                    for budget in [Budget::Core, Budget::Search, Budget::Graphql] {
                        // Aggregated totals
                        let (limit_sum, remaining_sum) = inner.token_pool.totals(budget).await;
                        metrics::BUDGET_LIMIT_TOTAL
                            .with_label_values(&[budget_label(budget)])
                            .set(limit_sum);
                        metrics::BUDGET_REMAINING_TOTAL
                            .with_label_values(&[budget_label(budget)])
                            .set(remaining_sum);

                        // Per-token gauges
                        let ids = inner.token_pool.token_ids().await;
                        for id in ids {
                            if let Some((limit, remaining)) =
                                inner.token_pool.get_numbers(budget, &id).await
                            {
                                metrics::RATE_LIMIT
                                    .with_label_values(&[&id, budget_label(budget)])
                                    .set(limit);
                                metrics::RATE_REMAINING
                                    .with_label_values(&[&id, budget_label(budget)])
                                    .set(remaining);
                            }
                        }
                    }

                    sleep(Duration::from_secs(5)).await;
                }
            });
        }

        Arc::new(LocalGithubBroker {
            inner,
            queues: senders,
        })
    }

    fn build_queues(
        bounds: &HashMap<(Budget, crate::model::Priority), usize>,
    ) -> (QueueSenders, QueueReceivers) {
        let mut senders = HashMap::new();
        let mut receivers_map: ReceiversMap = HashMap::new();

        for budget in [Budget::Core, Budget::Search, Budget::Graphql] {
            for priority in crate::model::Priority::ALL {
                let capacity = bounds.get(&(budget, priority)).cloned().unwrap_or(1024);
                let (tx, rx) = mpsc::channel(capacity);
                senders.insert((budget, priority), tx);
                let entry = receivers_map.entry(budget).or_insert((None, None, None));
                match priority {
                    crate::model::Priority::Critical => entry.0 = Some(rx),
                    crate::model::Priority::Normal => entry.1 = Some(rx),
                    crate::model::Priority::Backfill => entry.2 = Some(rx),
                }
            }
        }

        let mut receivers = Vec::new();
        for (budget, (crit, norm, back)) in receivers_map {
            receivers.push((budget, (crit.unwrap(), norm.unwrap(), back.unwrap())));
        }
        (senders, receivers)
    }
}

struct PendingEntry {
    waiters: Vec<oneshot::Sender<Result<BrokerResponse>>>,
}

struct Inner {
    http_exec: Arc<dyn HttpExec>,
    token_pool: TokenPool,
    cache: ResponseCache,
    pending: Mutex<HashMap<String, PendingEntry>>,
    inflight: Arc<Semaphore>,
    per_repo_limit: usize,
    per_repo: Mutex<HashMap<String, Arc<Semaphore>>>,
    backoff_base: Duration,
    backoff_max: Duration,
    jitter: f32,
}

impl Inner {
    async fn register_waiter(
        &self,
        key: &str,
    ) -> (oneshot::Receiver<Result<BrokerResponse>>, bool) {
        let (tx, rx) = oneshot::channel();
        let mut guard = self.pending.lock().await;
        match guard.get_mut(key) {
            Some(entry) => {
                entry.waiters.push(tx);
                (rx, false)
            }
            None => {
                guard.insert(key.to_string(), PendingEntry { waiters: vec![tx] });
                (rx, true)
            }
        }
    }

    async fn finish(&self, key: String, result: Result<BrokerResponse>) {
        let waiters = {
            let mut guard = self.pending.lock().await;
            guard
                .remove(&key)
                .map(|entry| entry.waiters)
                .unwrap_or_default()
        };
        match &result {
            Ok(response) => {
                for waiter in waiters {
                    let _ = waiter.send(Ok(response.clone()));
                }
            }
            Err(e) => {
                let err_msg = e.to_string();
                for waiter in waiters {
                    let _ = waiter.send(Err(anyhow::anyhow!("{}", err_msg)));
                }
            }
        }
    }

    async fn acquire_per_repo(&self, repo: &str) -> Arc<Semaphore> {
        let mut guard = self.per_repo.lock().await;
        if let Some(sema) = guard.get(repo) {
            return sema.clone();
        }
        let sema = Arc::new(Semaphore::new(self.per_repo_limit));
        guard.insert(repo.to_string(), sema.clone());
        sema
    }

    fn cache_key(&self, request: &GithubRequest) -> Option<String> {
        if request.method() == http::Method::GET {
            Some(request.key.clone())
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub struct LocalGithubBroker {
    inner: Arc<Inner>,
    queues: HashMap<(Budget, crate::model::Priority), mpsc::Sender<WorkItem>>,
}

impl LocalGithubBroker {
    async fn dispatch(&self, work: WorkItem) -> Result<()> {
        let budget = work.request.budget;
        let priority = work.request.priority;
        if let Some(sender) = self.queues.get(&(budget, priority)) {
            metrics::QUEUE_LENGTH
                .with_label_values(&[budget_label(budget), priority_label(priority)])
                .inc();
            metrics::PENDING
                .with_label_values(&[budget_label(budget), priority_label(priority)])
                .inc();
            sender
                .send(work)
                .await
                .map_err(|_| anyhow::anyhow!("queue closed"))?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("no queue for budget {:?}", budget))
        }
    }
}

impl GithubBroker for LocalGithubBroker {
    fn enqueue(
        &self,
        request: Request<Vec<u8>>,
        priority: crate::model::Priority,
    ) -> futures::future::BoxFuture<'static, Result<Response<Vec<u8>>>> {
        let broker = self.clone();
        async move {
            let mut gh_req = GithubRequest::new(request, priority)?;
            let cache_key = broker.inner.cache_key(&gh_req);
            let cached = if let Some(ref key) = cache_key {
                broker.inner.cache.get(key).await
            } else {
                None
            };

            if let Some(entry) = &cached {
                if let Some(etag) = &entry.etag {
                    gh_req
                        .headers_mut()
                        .insert(header::IF_NONE_MATCH, HeaderValue::from_str(etag)?);
                }
            }

            let (rx, should_dispatch) = broker.inner.register_waiter(gh_req.key()).await;

            if should_dispatch {
                broker
                    .dispatch(WorkItem {
                        key: gh_req.key().to_string(),
                        request: gh_req,
                        cached: cached.clone(),
                    })
                    .await?;
            }

            let response = rx.await??;
            Ok(response.into_response())
        }
        .boxed()
    }
}

#[derive(Clone, Debug)]
pub struct BrokerResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl BrokerResponse {
    pub fn from_http(resp: Response<Vec<u8>>) -> Self {
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .filter_map(|(k, v)| {
                v.to_str()
                    .ok()
                    .map(|val| (k.as_str().to_string(), val.to_string()))
            })
            .collect();
        let body = resp.into_body();
        Self {
            status,
            headers,
            body,
        }
    }

    pub fn into_response(self) -> Response<Vec<u8>> {
        let mut builder = Response::builder().status(self.status);
        for (key, value) in self.headers.iter() {
            if let (Ok(name), Ok(value)) = (
                header::HeaderName::from_bytes(key.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                builder.headers_mut().unwrap().append(name, value);
            }
        }
        builder.body(self.body).unwrap()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

async fn run_budget(
    inner: Arc<Inner>,
    budget: Budget,
    mut rx_critical: mpsc::Receiver<WorkItem>,
    mut rx_normal: mpsc::Receiver<WorkItem>,
    mut rx_backfill: mpsc::Receiver<WorkItem>,
    weights: [u32; 3],
) {
    loop {
        let mut processed = false;

        for (weight, rx) in [
            (weights[0], &mut rx_critical),
            (weights[1], &mut rx_normal),
            (weights[2], &mut rx_backfill),
        ] {
            for _ in 0..weight {
                match rx.try_recv() {
                    Ok(work) => {
                        processed = true;
                        metrics::QUEUE_LENGTH
                            .with_label_values(&[
                                budget_label(budget),
                                priority_label(work.request.priority),
                            ])
                            .dec();
                        process_work(inner.clone(), budget, work).await;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return,
                }
            }
        }

        if !processed {
            tokio::select! {
                Some(work) = rx_critical.recv() => {
                    metrics::QUEUE_LENGTH
                        .with_label_values(&[budget_label(budget), priority_label(work.request.priority)])
                        .dec();
                    process_work(inner.clone(), budget, work).await;
                },
                Some(work) = rx_normal.recv() => {
                    metrics::QUEUE_LENGTH
                        .with_label_values(&[budget_label(budget), priority_label(work.request.priority)])
                        .dec();
                    process_work(inner.clone(), budget, work).await;
                },
                Some(work) = rx_backfill.recv() => {
                    metrics::QUEUE_LENGTH
                        .with_label_values(&[budget_label(budget), priority_label(work.request.priority)])
                        .dec();
                    process_work(inner.clone(), budget, work).await;
                },
                else => break,
            }
        }
    }
}

async fn process_work(inner: Arc<Inner>, budget: Budget, work: WorkItem) {
    let key = work.key.clone();
    let mut attempt = 0;
    let request = work.request;
    loop {
        attempt += 1;
        match execute_once(inner.clone(), budget, &work.cached, request.clone()).await {
            Ok(response) => {
                inner.finish(key, Ok(response)).await;
                // Decrement pending on completion
                metrics::PENDING
                    .with_label_values(&[budget_label(budget), priority_label(request.priority)])
                    .dec();
                break;
            }
            Err(err) => {
                // Do not retry on most 4xx client errors (e.g., 404 Not Found),
                // except for 403/429 which may be rate/permission related.
                let mut retry_allowed = true;
                if let Some(http) = err.downcast_ref::<HttpStatusError>() {
                    let status = http.status;
                    if status.is_client_error()
                        && status != StatusCode::FORBIDDEN
                        && status != StatusCode::TOO_MANY_REQUESTS
                    {
                        retry_allowed = false;
                    }
                }

                if !retry_allowed || attempt >= 5 {
                    inner.finish(key, Err(err)).await;
                    metrics::PENDING
                        .with_label_values(&[
                            budget_label(budget),
                            priority_label(request.priority),
                        ])
                        .dec();
                    break;
                }

                warn!(
                    attempt,
                    budget = ?budget,
                    priority = %request.priority.as_str(),
                    request = %request.key(),
                    error = %err,
                    "GitHub request attempt failed"
                );
                let backoff = exponential_jitter_backoff(
                    inner.backoff_base,
                    attempt - 1,
                    inner.backoff_max,
                    inner.jitter,
                );
                metrics::RETRIES_TOTAL
                    .with_label_values(&[budget_label(budget), "error"])
                    .inc();
                sleep(backoff).await;
            }
        }
    }
}

async fn execute_once(
    inner: Arc<Inner>,
    budget: Budget,
    cached: &Option<CachedResponse>,
    mut request: GithubRequest,
) -> Result<BrokerResponse> {
    let permit = inner.inflight.clone().acquire_owned().await?;
    metrics::INFLIGHT
        .with_label_values(&[budget_label(budget)])
        .inc();

    let repo_key = extract_repo_key(&request);
    let repo_permit = if let Some(repo) = repo_key.clone() {
        let sem = inner.acquire_per_repo(&repo).await;
        Some((repo, sem.acquire_owned().await?))
    } else {
        None
    };

    let token = loop {
        match inner.token_pool.pick_token(budget).await {
            TokenSelection::Token(token) => break token,
            TokenSelection::Wait(wait) => {
                metrics::SLEEP_SECONDS
                    .with_label_values(&[budget_label(budget), "rate_limit"])
                    .inc_by(wait.as_secs());
                sleep(wait + Duration::from_secs(1)).await;
            }
        }
    };

    request.headers_mut().insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("token {}", token.secret))?,
    );

    metrics::SCHEDULED_TOTAL
        .with_label_values(&[budget_label(budget), priority_label(request.priority)])
        .inc();

    let start = std::time::Instant::now();
    let response = inner.http_exec.execute(request.request()).await;
    metrics::INFLIGHT
        .with_label_values(&[budget_label(budget)])
        .dec();
    drop(permit);
    drop(repo_permit);

    match response {
        Ok(resp) => {
            metrics::LATENCY
                .with_label_values(&[budget_label(budget)])
                .observe(start.elapsed().as_secs_f64());
            let status = resp.status();
            metrics::REQUESTS_TOTAL
                .with_label_values(&[budget_label(budget), &token.id, status_class(status)])
                .inc();

            if let Some(update) = parse_rate_limit(resp.headers()) {
                inner
                    .token_pool
                    .update(budget, &token.id, update.clone())
                    .await;
                metrics::RATE_LIMIT
                    .with_label_values(&[&token.id, budget_label(budget)])
                    .set(update.limit);
                metrics::RATE_REMAINING
                    .with_label_values(&[&token.id, budget_label(budget)])
                    .set(update.remaining);

                // Update aggregated capacity gauges per budget
                let (limit_sum, remaining_sum) = inner.token_pool.totals(budget).await;
                metrics::BUDGET_LIMIT_TOTAL
                    .with_label_values(&[budget_label(budget)])
                    .set(limit_sum);
                metrics::BUDGET_REMAINING_TOTAL
                    .with_label_values(&[budget_label(budget)])
                    .set(remaining_sum);
            }

            if status == StatusCode::NOT_MODIFIED {
                if let Some(entry) = cached {
                    metrics::CACHE_HITS
                        .with_label_values(&[budget_label(budget)])
                        .inc();
                    return Ok(BrokerResponse {
                        status: entry.status,
                        headers: entry.headers.clone(),
                        body: entry.body.clone(),
                    });
                }
            }

            let headers = resp.headers().clone();

            if status.is_success() {
                let cache_key = request.key().to_string();
                let response = BrokerResponse::from_http(resp);
                if request.method() == http::Method::GET {
                    metrics::CACHE_MISSES
                        .with_label_values(&[budget_label(budget)])
                        .inc();
                    if let Some(etag) = response.header("etag").map(|s| s.to_string()) {
                        let cached_response = CachedResponse {
                            etag: Some(etag),
                            body: response.body.clone(),
                            status: response.status,
                            headers: response.headers.clone(),
                            stored_at: std::time::Instant::now(),
                        };
                        inner.cache.put(cache_key, cached_response).await;
                    }
                }

                let cost = if budget == Budget::Graphql {
                    extract_graphql_cost(&response).unwrap_or(1)
                } else {
                    1
                };
                inner
                    .token_pool
                    .consume(budget, &token.id, cost as i64)
                    .await;

                // Refresh per-token gauges after consumption (GraphQL cost may not include headers)
                if let Some((limit, remaining)) =
                    inner.token_pool.get_numbers(budget, &token.id).await
                {
                    metrics::RATE_LIMIT
                        .with_label_values(&[&token.id, budget_label(budget)])
                        .set(limit);
                    metrics::RATE_REMAINING
                        .with_label_values(&[&token.id, budget_label(budget)])
                        .set(remaining);
                }

                // Refresh aggregated capacity after consumption (handles cases with missing headers)
                let (limit_sum, remaining_sum) = inner.token_pool.totals(budget).await;
                metrics::BUDGET_LIMIT_TOTAL
                    .with_label_values(&[budget_label(budget)])
                    .set(limit_sum);
                metrics::BUDGET_REMAINING_TOTAL
                    .with_label_values(&[budget_label(budget)])
                    .set(remaining_sum);

                return Ok(response);
            }

            if let Some(retry) = parse_retry_after(&headers) {
                let rate_info = parse_rate_limit(&headers);
                warn!(
                    status = %status,
                    request = %request.key(),
                    budget = ?budget,
                    priority = %request.priority.as_str(),
                    github_request_id = headers
                        .get("x-github-request-id")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("-"),
                    retry_after_seconds = retry.wait.as_secs(),
                    rate_limit_remaining = rate_info.as_ref().map(|data| data.remaining),
                    rate_limit_reset = rate_info
                        .as_ref()
                        .map(|data| data.reset.timestamp()),
                    "GitHub responded with retryable status"
                );
                metrics::SLEEP_SECONDS
                    .with_label_values(&[budget_label(budget), retry.reason])
                    .inc_by(retry.wait.as_secs());
                sleep(retry.wait + Duration::from_secs(1)).await;
                return Err(anyhow::anyhow!("retry after"));
            }

            if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
                let rate_info = parse_rate_limit(&headers);
                warn!(
                    status = %status,
                    request = %request.key(),
                    budget = ?budget,
                    priority = %request.priority.as_str(),
                    github_request_id = headers
                        .get("x-github-request-id")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("-"),
                    rate_limit_remaining = rate_info.as_ref().map(|data| data.remaining),
                    rate_limit_reset = rate_info
                        .as_ref()
                        .map(|data| data.reset.timestamp()),
                    "GitHub returned secondary rate limit response"
                );
                metrics::SLEEP_SECONDS
                    .with_label_values(&[budget_label(budget), "secondary_limit"])
                    .inc_by(3);
                sleep(Duration::from_secs(3)).await;
                return Err(anyhow::anyhow!("secondary rate limit"));
            }

            let rate_info = parse_rate_limit(&headers);
            let request_id = headers
                .get("x-github-request-id")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("-");
            let body_snapshot = resp.body().clone();
            let body_preview = body_preview(&body_snapshot);
            warn!(
                status = %status,
                request = %request.key(),
                budget = ?budget,
                priority = %request.priority.as_str(),
                github_request_id = request_id,
                rate_limit_remaining = rate_info.as_ref().map(|data| data.remaining),
                rate_limit_reset = rate_info
                    .as_ref()
                    .map(|data| data.reset.timestamp()),
                body_preview = %body_preview,
                "GitHub returned error response"
            );
            Err(HttpStatusError::new(status).into())
        }
        Err(err) => Err(err),
    }
}

fn body_preview(body: &[u8]) -> String {
    if body.is_empty() {
        return String::new();
    }
    let text = String::from_utf8_lossy(body);
    truncate_str(&text, 256)
}

fn truncate_str(value: &str, limit: usize) -> String {
    if value.is_empty() {
        return String::new();
    }
    let mut truncated: String = value.chars().take(limit).collect();
    if truncated.len() < value.len() {
        truncated.push('…');
    }
    truncated
}

fn extract_graphql_cost(response: &BrokerResponse) -> Option<u64> {
    let body: serde_json::Value = serde_json::from_slice(&response.body).ok()?;
    let data = body.get("data")?;
    let rate = data.get("rateLimit")?;
    let cost = rate.get("cost")?.as_u64()?;
    Some(cost.max(1))
}

fn extract_repo_key(request: &GithubRequest) -> Option<String> {
    let path = request.uri().path().trim_start_matches('/');
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() >= 3 && segments[0] == "repos" {
        Some(format!("{}/{}", segments[1], segments[2]))
    } else {
        None
    }
}

fn status_class(status: StatusCode) -> &'static str {
    match status.as_u16() {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        _ => "5xx",
    }
}

fn budget_label(budget: Budget) -> &'static str {
    match budget {
        Budget::Core => "core",
        Budget::Search => "search",
        Budget::Graphql => "graphql",
    }
}

fn priority_label(priority: crate::model::Priority) -> &'static str {
    match priority {
        crate::model::Priority::Critical => "critical",
        crate::model::Priority::Normal => "normal",
        crate::model::Priority::Backfill => "backfill",
    }
}
