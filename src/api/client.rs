// borrowed from: https://sivakarasala.github.io/building-ai-agents/rust/01-setup-and-first-call.html#the-openai-chat-completions-api
use color_eyre::{
    Result,
    eyre::{WrapErr, eyre},
};
use reqwest::Client;

use super::types::{ChatCompletionRequest, ChatCompletionResponse};

const API_URL: &str = "https://api.openai.com/v1/chat/completions";

pub struct OpenAIClient {
    client: Client,
    api_key: String,
}

impl OpenAIClient {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
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
            .wrap_err("Failed to send request to OpenAI")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(eyre!("OpenAI API error ({}): {}", status, body));
        }

        let body = response
            .json::<ChatCompletionResponse>()
            .await
            .wrap_err("Failed to parse OpenAI response")?;

        Ok(body)
    }
}
