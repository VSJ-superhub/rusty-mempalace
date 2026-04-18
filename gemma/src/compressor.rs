use async_trait::async_trait;

#[async_trait]
pub trait Compressor: Send + Sync {
    async fn compress(&self, text: &str) -> anyhow::Result<String>;
}

pub struct NoopCompressor;

#[async_trait]
impl Compressor for NoopCompressor {
    async fn compress(&self, text: &str) -> anyhow::Result<String> {
        Ok(text.to_string())
    }
}

pub struct WithFallback {
    inner: Box<dyn Compressor + Send + Sync>,
}

impl WithFallback {
    pub fn new(inner: Box<dyn Compressor + Send + Sync>) -> Self {
        WithFallback { inner }
    }
}

#[async_trait]
impl Compressor for WithFallback {
    async fn compress(&self, text: &str) -> anyhow::Result<String> {
        match self.inner.compress(text).await {
            Ok(result) => Ok(result),
            Err(e) => {
                tracing::warn!("Compressor failed, returning original content: {}", e);
                Ok(text.to_string())
            }
        }
    }
}
