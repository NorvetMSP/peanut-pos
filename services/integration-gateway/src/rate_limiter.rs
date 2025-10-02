use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

// Redis dependencies (only used by Redis implementation)
use redis::aio::ConnectionManager;
use redis::AsyncCommands;

#[derive(Debug, Clone, Copy)]
pub struct RateDecision {
    pub allowed: bool,
    pub current: i64,
}

#[async_trait]
pub trait RateLimiterEngine: Send + Sync {
    async fn check(&self, key: &str, limit: u32) -> Result<RateDecision>;
}

// ---------------- Redis Implementation ----------------

#[derive(Clone)]
pub struct RedisRateLimiter {
    manager: ConnectionManager,
    window_secs: u64,
    prefix: String,
}

impl RedisRateLimiter {
    pub async fn new(redis_url: &str, window_secs: u64, prefix: String) -> Result<Self> {
        let client = redis::Client::open(redis_url).context("Failed to create Redis client")?;
        let manager = ConnectionManager::new(client)
            .await
            .context("Failed to create Redis connection manager")?;
        Ok(Self { manager, window_secs, prefix })
    }
}

#[async_trait]
impl RateLimiterEngine for RedisRateLimiter {
    async fn check(&self, key: &str, limit: u32) -> Result<RateDecision> {
        let redis_key = format!("{}:{}", self.prefix, key);
        let mut conn = self.manager.clone();
        let current: i64 = conn.incr(&redis_key, 1).await?;
        if current == 1 {
            let _: () = conn.expire(&redis_key, self.window_secs as i64).await?;
        }
        let allowed = current <= limit as i64;
        Ok(RateDecision { allowed, current })
    }
}

// ---------------- In-Memory Implementation (Tests) ----------------

#[derive(Clone)]
pub struct InMemoryRateLimiter {
    inner: Arc<Mutex<HashMap<String, (i64, std::time::Instant)>>>,
    window_secs: u64,
}

impl InMemoryRateLimiter {
    pub fn new(window_secs: u64) -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())), window_secs }
    }
}

#[async_trait]
impl RateLimiterEngine for InMemoryRateLimiter {
    async fn check(&self, key: &str, limit: u32) -> Result<RateDecision> {
        let mut guard = self.inner.lock().await;
        let now = std::time::Instant::now();
        let window = std::time::Duration::from_secs(self.window_secs);
        let entry = guard.entry(key.to_string()).or_insert((0, now));
        if now.duration_since(entry.1) >= window {
            *entry = (0, now);
        }
        entry.0 += 1;
        let current = entry.0;
        let allowed = current <= limit as i64;
        Ok(RateDecision { allowed, current })
    }
}

// Convenience enum wrapper if needed in future (not currently used)
#[allow(dead_code)]
pub enum RateLimiter {
    Redis(RedisRateLimiter),
    Memory(InMemoryRateLimiter),
}

impl RateLimiter {
    pub async fn redis(redis_url: &str, window_secs: u64, prefix: String) -> Result<Self> {
        Ok(RateLimiter::Redis(RedisRateLimiter::new(redis_url, window_secs, prefix).await?))
    }
    pub fn memory(window_secs: u64) -> Self { RateLimiter::Memory(InMemoryRateLimiter::new(window_secs)) }
}
