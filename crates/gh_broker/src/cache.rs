use std::time::{Duration, Instant};

use lru::LruCache;
use tokio::sync::Mutex;

#[derive(Clone, Debug)]
pub struct CachedResponse {
    pub etag: Option<String>,
    pub body: Vec<u8>,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub stored_at: Instant,
}

impl CachedResponse {
    pub fn is_fresh(&self, ttl: Duration) -> bool {
        self.stored_at.elapsed() < ttl
    }
}

#[derive(Clone)]
pub struct ResponseCache {
    inner: std::sync::Arc<Mutex<LruCache<String, CachedResponse>>>,
    ttl: Duration,
}

impl ResponseCache {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            inner: std::sync::Arc::new(Mutex::new(LruCache::new(capacity.try_into().unwrap()))),
            ttl,
        }
    }

    pub async fn get(&self, key: &str) -> Option<CachedResponse> {
        let mut guard = self.inner.lock().await;
        guard
            .get(key)
            .cloned()
            .filter(|entry| entry.is_fresh(self.ttl))
    }

    pub async fn put(&self, key: String, value: CachedResponse) {
        let mut guard = self.inner.lock().await;
        guard.put(key, value);
    }
}
