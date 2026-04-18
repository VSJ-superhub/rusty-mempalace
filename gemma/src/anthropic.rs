use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::Compressor;

#[derive(Debug, Clone)]
pub struct AnthropicCompressor {
    client: Client,
    api_key: String,
    model: String,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<AnthropicMessage>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    text: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
}

impl AnthropicCompressor {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }
}

#[async_trait::async_trait]
impl Compressor for AnthropicCompressor {
    async fn compress(&self, input: &str) -> anyhow::Result<String> {
        let request = AnthropicRequest {
            model: self.model.clone(),
            max_tokens: 1024,
            system: "Compress the following memory fragment into a concise, information-dense summary. Preserve all key facts, entities, and relationships. Output only the compressed text with no preamble.".to_string(),
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: input.to_string(),
            }],
        };

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error {}: {}", status, body);
        }

        let anthropic_response: AnthropicResponse = response.json().await?;
        anthropic_response
            .content
            .into_iter()
            .next()
            .map(|c| c.text)
            .ok_or_else(|| anyhow::anyhow!("Anthropic API returned no content"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    struct AnthropicCompressorWithBase {
        client: Client,
        base_url: String,
        api_key: String,
        model: String,
    }

    impl AnthropicCompressorWithBase {
        fn new(base_url: impl Into<String>, api_key: impl Into<String>, model: impl Into<String>) -> Self {
            Self {
                client: Client::new(),
                base_url: base_url.into(),
                api_key: api_key.into(),
                model: model.into(),
            }
        }

        async fn compress(&self, input: &str) -> anyhow::Result<String> {
            let url = format!("{}/v1/messages", self.base_url);
            let request = AnthropicRequest {
                model: self.model.clone(),
                max_tokens: 1024,
                system: "Compress the following memory fragment into a concise, information-dense summary. Preserve all key facts, entities, and relationships. Output only the compressed text with no preamble.".to_string(),
                messages: vec![AnthropicMessage {
                    role: "user".to_string(),
                    content: input.to_string(),
                }],
            };

            let response = self
                .client
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&request)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                anyhow::bail!("Anthropic API error {}: {}", status, body);
            }

            let anthropic_response: AnthropicResponse = response.json().await?;
            anthropic_response
                .content
                .into_iter()
                .next()
                .map(|c| c.text)
                .ok_or_else(|| anyhow::anyhow!("Anthropic API returned no content"))
        }
    }

    #[tokio::test]
    async fn test_compress_success() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/messages")
            .match_header("x-api-key", "test-key")
            .match_header("anthropic-version", "2023-06-01")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"content":[{"type":"text","text":"Compressed: user prefers dark mode, uses VSCode."}]}"#)
            .create_async()
            .await;

        let compressor = AnthropicCompressorWithBase::new(server.url(), "test-key", "claude-haiku-4-5-20251001");
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
            .mock("POST", "/v1/messages")
            .with_status(401)
            .with_body(r#"{"error":{"type":"authentication_error","message":"Invalid API key"}}"#)
            .create_async()
            .await;

        let compressor = AnthropicCompressorWithBase::new(server.url(), "bad-key", "claude-haiku-4-5-20251001");
        let result = compressor.compress("some input").await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("401"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_compress_empty_content() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/messages")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"content":[]}"#)
            .create_async()
            .await;

        let compressor = AnthropicCompressorWithBase::new(server.url(), "test-key", "claude-haiku-4-5-20251001");
        let result = compressor.compress("input").await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no content"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_compress_sends_correct_model() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/messages")
            .match_header("x-api-key", "my-key")
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "model": "claude-haiku-4-5-20251001"
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"content":[{"type":"text","text":"summary"}]}"#)
            .create_async()
            .await;

        let compressor = AnthropicCompressorWithBase::new(server.url(), "my-key", "claude-haiku-4-5-20251001");
        let result = compressor.compress("input text").await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "summary");
        mock.assert_async().await;
    }
}
