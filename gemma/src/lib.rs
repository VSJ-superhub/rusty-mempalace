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
            Ok(out) => Ok(out),
            Err(e) => {
                warn!("Compressor error, falling back to noop: {}", e);
                Ok(content.to_string())
            }
        }
    }
}

// ── Config factory ────────────────────────────────────────────────────────────

pub fn compressor_from_config(config: &CompressionConfig) -> Box<dyn Compressor> {
    match config.backend.as_deref().unwrap_or("none") {
        "ollama" => {
            let mut c = OllamaCompressor::new(
                config.model.clone().unwrap_or_else(|| "gemma:1b".to_string()),
            );
            if let Some(url) = &config.base_url {
                c = c.with_base_url(url.clone());
            }
            Box::new(WithFallback::new(c))
        }
        "openai" => Box::new(WithFallback::new(OpenAICompatibleCompressor {
            model: config.model.clone().unwrap_or_else(|| "gpt-3.5-turbo".to_string()),
            base_url: config
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com".to_string()),
            api_key: config.api_key.clone().unwrap_or_default(),
        })),
        "anthropic" => Box::new(WithFallback::new(AnthropicCompressor::new(
            config.api_key.clone().unwrap_or_default(),
            config
                .model
                .clone()
                .unwrap_or_else(|| "claude-haiku-4-5-20251001".to_string()),
        ))),
        _ => Box::new(NoopCompressor),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn noop_returns_input_unchanged() {
        let c = NoopCompressor;
        let result = c.compress("hello world").await.unwrap();
        assert_eq!(result, "hello world");
    }

    #[tokio::test]
    async fn ollama_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "response": "compressed fact" })),
            )
            .mount(&server)
            .await;

        let c = OllamaCompressor::new("gemma:1b").with_base_url(server.uri());
        let result = c.compress("long content here").await.unwrap();
        assert_eq!(result, "compressed fact");
    }

    #[tokio::test]
    async fn ollama_non200_falls_back_to_noop() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let c = WithFallback::new(
            OllamaCompressor::new("gemma:1b").with_base_url(server.uri()),
        );
        let result = c.compress("original content").await.unwrap();
        assert_eq!(result, "original content");
    }

    #[tokio::test]
    async fn openai_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "choices": [{ "message": { "content": "openai compressed" } }]
                })),
            )
            .mount(&server)
            .await;

        let c = OpenAICompatibleCompressor {
            model: "gpt-3.5-turbo".to_string(),
            base_url: server.uri(),
            api_key: "test-key".to_string(),
        };
        let result = c.compress("some content").await.unwrap();
        assert_eq!(result, "openai compressed");
    }

    #[tokio::test]
    async fn openai_non200_falls_back_to_noop() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let c = WithFallback::new(OpenAICompatibleCompressor {
            model: "gpt-3.5-turbo".to_string(),
            base_url: server.uri(),
            api_key: "bad-key".to_string(),
        });
        let result = c.compress("original").await.unwrap();
        assert_eq!(result, "original");
    }

    #[test]
    fn config_parses_all_fields() {
        let toml = r#"
[compression]
backend = "openai"
model = "gpt-4o-mini"
base_url = "https://api.groq.com"
api_key = "sk-test"
"#;
        let config = CompressionConfig::from_toml(toml).unwrap();
        assert_eq!(config.backend.as_deref(), Some("openai"));
        assert_eq!(config.model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(config.base_url.as_deref(), Some("https://api.groq.com"));
        assert_eq!(config.api_key.as_deref(), Some("sk-test"));
    }

    #[test]
    fn config_defaults_to_none_backend() {
        let config = CompressionConfig::from_toml("[compression]\n").unwrap();
        assert_eq!(config.backend, None);
    }

    #[tokio::test]
    async fn factory_none_backend_returns_noop() {
        let config = CompressionConfig::default();
        let c = compressor_from_config(&config);
        let result = c.compress("test input").await.unwrap();
        assert_eq!(result, "test input");
    }
}
