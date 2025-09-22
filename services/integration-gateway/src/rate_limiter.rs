use anyhow::{Context, Result};
use redis::aio::ConnectionManager;
use redis::AsyncCommands;

#[derive(Clone)]
pub struct RateLimiter {
    manager: ConnectionManager,
    window_secs: u64,
    prefix: String,
}

#[derive(Debug, Clone, Copy)]
pub struct RateDecision {
    pub allowed: bool,
    pub current: i64,
}

impl RateLimiter {
    pub async fn new(redis_url: &str, window_secs: u64, prefix: String) -> Result<Self> {
        let client = redis::Client::open(redis_url).context("Failed to create Redis client")?;
        let manager = ConnectionManager::new(client)
            .await
            .context("Failed to create Redis connection manager")?;
        Ok(Self {
            manager,
            window_secs,
            prefix,
        })
    }

    pub async fn check(&self, key: &str, limit: u32) -> Result<RateDecision> {
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
