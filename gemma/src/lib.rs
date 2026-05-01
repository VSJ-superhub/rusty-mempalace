use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use tracing::warn;

mod anthropic;
pub use anthropic::AnthropicCompressor;

#[async_trait]
pub trait Compressor: Send + Sync {
    async fn compress(&self, content: &str) -> Result<String>;
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct CompressionConfig {
    pub backend: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfigFile {
    compression: Option<CompressionConfig>,
}

impl CompressionConfig {
    pub fn from_toml(toml_str: &str) -> Result<Self> {
        let file: ConfigFile = toml::from_str(toml_str)?;
        Ok(file.compression.unwrap_or_default())
    }
}

// ── NoopCompressor ────────────────────────────────────────────────────────────

pub struct NoopCompressor;

#[async_trait]
impl Compressor for NoopCompressor {
    async fn compress(&self, content: &str) -> Result<String> {
        Ok(content.to_string())
    }
}

// ── OllamaCompressor ──────────────────────────────────────────────────────────

pub struct OllamaCompressor {
    pub model: String,
    pub base_url: String,
}

impl OllamaCompressor {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            base_url: "http://localhost:11434".to_string(),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}

#[async_trait]
impl Compressor for OllamaCompressor {
    async fn compress(&self, content: &str) -> Result<String> {
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "model": self.model,
            "prompt": format!(
                "Compress the following memory fragment into a concise fact. \
                 Preserve key information:\n\n{}",
                content
            ),
            "stream": false
        });

        let resp = client
            .post(format!("{}/api/generate", self.base_url))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Ollama returned {}", resp.status());
        }

        let data: serde_json::Value = resp.json().await?;
        Ok(data["response"].as_str().unwrap_or(content).to_string())
    }
}

// ── OpenAICompatibleCompressor ────────────────────────────────────────────────

pub struct OpenAICompatibleCompressor {
    pub model: String,
    pub base_url: String,
    pub api_key: String,
}

#[async_trait]
impl Compressor for OpenAICompatibleCompressor {
    async fn compress(&self, content: &str) -> Result<String> {
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{
                "role": "user",
                "content": format!(
                    "Compress the following memory fragment into a concise fact. \
                     Preserve key information:\n\n{}",
                    content
                )
            }]
        });

        let resp = client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("OpenAI-compatible API returned {}", resp.status());
        }

        let data: serde_json::Value = resp.json().await?;
        Ok(data["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or(content)
            .to_string())
    }
}

// ── WithFallback wrapper ──────────────────────────────────────────────────────

pub struct WithFallback<C: Compressor> {
    inner: C,
}

impl<C: Compressor> WithFallback<C> {
    pub fn new(inner: C) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<C: Compressor> Compressor for WithFallback<C> {
    async fn compress(&self, content: &str) -> Result<String> {
        match self.inner.compress(content).await {
            Ok(result) => Ok(result),
            Err(e) => {
                warn!("Compressor failed, falling back to noop: {}", e);
                Ok(content.to_string())
            }
        }
    }
}

// ── Factory ───────────────────────────────────────────────────────────────────

pub fn build_compressor(
    config: &CompressionConfig,
) -> Box<dyn Compressor> {
    match config.backend.as_deref().unwrap_or("ollama") {
        "ollama" => {
            let model = config.model.clone().unwrap_or_else(|| "gemma3".to_string());
            let mut c = OllamaCompressor::new(model);
            if let Some(url) = &config.base_url {
                c = c.with_base_url(url.clone());
            }
            Box::new(WithFallback::new(c))
        }
        "openai" => {
            let c = OpenAICompatibleCompressor {
                model: config.model.clone().unwrap_or_else(|| "gpt-4o-mini".to_string()),
                base_url: config
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.openai.com".to_string()),
                api_key: config.api_key.clone().unwrap_or_default(),
            };
            Box::new(WithFallback::new(c))
        }
        "anthropic" => {
            let c = AnthropicCompressor::new(
                config.api_key.clone().unwrap_or_default(),
                config.model.clone().unwrap_or_else(|| "claude-haiku-4-5-20251001".to_string()),
            );
            Box::new(WithFallback::new(c))
        }
        _ => Box::new(NoopCompressor),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_returns_input_unchanged() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let c = NoopCompressor;
            c.compress("hello world").await.unwrap()
        });
        assert_eq!(result, "hello world");
    }

    #[test]
    fn config_parses_backend_fields() {
        let toml = r#"
[compression]
backend = "openai"
model = "gpt-4o"
base_url = "https://api.openai.com"
api_key = "sk-test"
"#;
        let cfg = CompressionConfig::from_toml(toml).unwrap();
        assert_eq!(cfg.backend.as_deref(), Some("openai"));
        assert_eq!(cfg.model.as_deref(), Some("gpt-4o"));
        assert_eq!(cfg.base_url.as_deref(), Some("https://api.openai.com"));
        assert_eq!(cfg.api_key.as_deref(), Some("sk-test"));
    }

    #[tokio::test]
    async fn fallback_on_error() {
        struct AlwaysFails;

        #[async_trait]
        impl Compressor for AlwaysFails {
            async fn compress(&self, _content: &str) -> Result<String> {
                anyhow::bail!("simulated failure")
            }
        }

        let wrapped = WithFallback::new(AlwaysFails);
        let result = wrapped.compress("original content").await.unwrap();
        assert_eq!(result, "original content");
    }

    #[tokio::test]
    async fn ollama_non_200_triggers_fallback() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/api/generate")
            .with_status(503)
            .create_async()
            .await;

        let compressor = OllamaCompressor::new("gemma3").with_base_url(server.url());
        let wrapped = WithFallback::new(compressor);
        let result = wrapped.compress("test content").await.unwrap();
        assert_eq!(result, "test content");
    }

    #[tokio::test]
    async fn config_parses_backend_fields_async() {
        let toml = r#"
[compression]
backend = "anthropic"
model = "claude-haiku-4-5"
api_key = "test-key"
"#;
        let cfg = CompressionConfig::from_toml(toml).unwrap();
        assert_eq!(cfg.backend.as_deref(), Some("anthropic"));
        assert_eq!(cfg.model.as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(cfg.api_key.as_deref(), Some("test-key"));
    }
}
