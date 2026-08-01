use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::Client;

use super::sse::StreamChunk;
use super::types::{ChatCompletionRequest, ChatCompletionResponse};

const API_URL: &str = "https://opencode.ai/zen/v1/chat/completions";

pub struct OpenAIClient {
    client: Client,
    api_key: String,
}

impl OpenAIClient {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key,
        }
    }

    /// Make a non-streaming chat completion request.
    pub async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse> {
        let response = self
            .client
            .post(API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send chat completion request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Chat completion error: ({}): {}", status, body);
        }

        let body = response
            .json::<ChatCompletionResponse>()
            .await
            .context("Failed to parse chat completion response")?;

        Ok(body)
    }

    /// Streaming request - returns chunks via a callback
    pub async fn chat_completion_stream(
        &self,
        mut request: ChatCompletionRequest,
        mut on_chunk: impl FnMut(StreamChunk),
    ) -> Result<()> {
        request.stream = Some(true);

        let response = self
            .client
            .post(API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send streaming request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Chat completion stream error ({}): {}", status, body);
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.context("Stream read error")?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            // process complete lines
            while let Some(line_end) = buffer.find('\n') {
                let line = buffer[..line_end].trim().to_string();
                buffer = buffer[line_end + 1..].to_string();

                if line.is_empty() {
                    continue;
                }

                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        return Ok(());
                    }

                    match serde_json::from_str::<StreamChunk>(data) {
                        Ok(chunk) => on_chunk(chunk),
                        Err(e) => {
                            eprintln!("Failed to parse sse chunk: {e}");
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
