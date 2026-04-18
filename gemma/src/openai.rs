use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::Compressor;

#[derive(Debug, Clone)]
pub struct OpenAICompatibleCompressor {
    client: Client,
    base_url: String,
    api_key: String,
    model: String,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}

#[derive(Deserialize)]
struct ChatMessageResponse {
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

impl OpenAICompatibleCompressor {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }
}

#[async_trait::async_trait]
impl Compressor for OpenAICompatibleCompressor {
    async fn compress(&self, input: &str) -> anyhow::Result<String> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: "Compress the following memory fragment into a concise, information-dense summary. Preserve all key facts, entities, and relationships. Output only the compressed text with no preamble.".to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: input.to_string(),
                },
            ],
        };

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API error {}: {}", status, body);
        }

        let chat_response: ChatResponse = response.json().await?;
        chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| anyhow::anyhow!("OpenAI API returned no choices"))
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
            .mock("POST", "/v1/chat/completions")
            .match_header("authorization", "Bearer test-key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"choices":[{"message":{"role":"assistant","content":"Compressed: user prefers dark mode, uses VSCode."}}]}"#)
            .create_async()
            .await;

        let compressor = OpenAICompatibleCompressor::new(server.url(), "test-key", "gpt-4o-mini");
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
            .mock("POST", "/v1/chat/completions")
            .with_status(401)
            .with_body(r#"{"error":{"message":"Invalid API key"}}"#)
            .create_async()
            .await;

        let compressor = OpenAICompatibleCompressor::new(server.url(), "bad-key", "gpt-4o-mini");
        let result = compressor
            .compress("some input")
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("401"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_compress_sends_correct_model() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .match_header("authorization", "Bearer my-key")
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "model": "gpt-4o"
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"choices":[{"message":{"role":"assistant","content":"summary"}}]}"#)
            .create_async()
            .await;

        let compressor = OpenAICompatibleCompressor::new(server.url(), "my-key", "gpt-4o");
        let result = compressor.compress("input text").await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "summary");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_compress_empty_choices() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"choices":[]}"#)
            .create_async()
            .await;

        let compressor = OpenAICompatibleCompressor::new(server.url(), "test-key", "gpt-4o-mini");
        let result = compressor.compress("input").await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no choices"));
        mock.assert_async().await;
    }
}
