use serde::Deserialize;
use crate::ollama;
use crate::openai;
use crate::anthropic;
use crate::compressor::{Compressor, NoopCompressor, WithFallback};

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "backend", rename_all = "snake_case")]
pub enum Backend {
    Ollama {
        #[serde(default = "default_ollama_url")]
        base_url: String,
        #[serde(default = "default_ollama_model")]
        model: String,
    },
    OpenAiCompatible {
        base_url: String,
        model: String,
        #[serde(default)]
        api_key: Option<String>,
    },
    Anthropic {
        #[serde(default = "default_anthropic_model")]
        model: String,
        api_key: String,
    },
    Noop,
}

impl Default for Backend {
    fn default() -> Self {
        Backend::Ollama {
            base_url: default_ollama_url(),
            model: default_ollama_model(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CompressionConfig {
    #[serde(flatten, default)]
    pub backend: Backend,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        CompressionConfig {
            backend: Backend::default(),
            max_tokens: default_max_tokens(),
        }
    }
}

fn default_ollama_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_ollama_model() -> String {
    "gemma3:4b".to_string()
}

fn default_anthropic_model() -> String {
    "claude-haiku-4-5-20251001".to_string()
}

fn default_max_tokens() -> usize {
    512
}

pub fn compressor_from_config(config: CompressionConfig) -> Box<dyn Compressor + Send + Sync> {
    let inner: Box<dyn Compressor + Send + Sync> = match config.backend {
        Backend::Ollama { base_url, model } => {
            Box::new(ollama::OllamaCompressor::new(base_url, model, config.max_tokens))
        }
        Backend::OpenAiCompatible { base_url, model, api_key } => {
            Box::new(openai::OpenAICompatibleCompressor::new(base_url, model, api_key, config.max_tokens))
        }
        Backend::Anthropic { model, api_key } => {
            Box::new(anthropic::AnthropicCompressor::new(model, api_key, config.max_tokens))
        }
        Backend::Noop => Box::new(NoopCompressor),
    };
    Box::new(WithFallback::new(inner))
}
