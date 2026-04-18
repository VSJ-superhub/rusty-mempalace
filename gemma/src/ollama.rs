use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::Compressor;

#[derive(Debug, Clone)]
pub struct OllamaCompressor {
    client: Client,
    base_url: String,
    model: String,
}

#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    prompt: String,
    stream: bool,
}

#[derive(Deserialize)]
struct OllamaResponse {
    response: String,
}

impl OllamaCompressor {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: "http://localhost:11434".to_string(),
            model: model.into(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

impl Default for OllamaCompressor {
    fn default() -> Self {
        Self::new("gemma3")
    }
}

#[async_trait::async_trait]
impl Compressor for OllamaCompressor {
    async fn compress(&self, input: &str) -> anyhow::Result<String> {
        let url = format!("{}/api/generate", self.base_url);
        let request = OllamaRequest {
            model: self.model.clone(),
            prompt: format!(
                "Compress the following memory fragment into a concise, information-dense summary. Preserve all key facts, entities, and relationships. Output only the compressed text with no preamble:\n\n{}",
                input
            ),
            stream: false,
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama API error {}: {}", status, body);
        }

        let ollama_response: OllamaResponse = response.json().await?;
        Ok(ollama_response.response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[tokio::test]
    async fn test_compress_success() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/generate")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"response":"Compressed: user prefers dark mode, uses VSCode."}"#)
            .create_async()
            .await;

        let compressor = OllamaCompressor::new("gemma3").with_base_url(server.url());
        let result = compressor
            .compress("The user told me they prefer dark mode themes in their editor. They use VSCode as their primary development environment.")
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Compressed: user prefers dark mode, uses VSCode.");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_compress_server_error() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/api/generate")
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let compressor = OllamaCompressor::new("gemma3").with_base_url(server.url());
        let result = compressor.compress("some input").await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("500"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_compress_connection_refused() {
        let compressor = OllamaCompressor::new("gemma3")
            .with_base_url("http://127.0.0.1:1");
        let result = compressor.compress("some input").await;
        assert!(result.is_err());
    }
}
