use anyhow::Result;

#[async_trait::async_trait]
pub trait PaymentGateway: Send + Sync {
    async fn void(&self, provider: &str, provider_ref: &str) -> Result<Option<String>>;
    async fn refund(&self, provider: &str, provider_ref: &str) -> Result<Option<String>>;
}

pub struct StubGateway;

impl StubGateway { pub fn new() -> Self { Self } }

#[async_trait::async_trait]
impl PaymentGateway for StubGateway {
    async fn void(&self, _provider: &str, provider_ref: &str) -> Result<Option<String>> {
        Ok(Some(format!("{}-void", provider_ref)))
    }
    async fn refund(&self, _provider: &str, provider_ref: &str) -> Result<Option<String>> {
        Ok(Some(format!("{}-refund", provider_ref)))
    }
}
