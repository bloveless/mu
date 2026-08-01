use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::agent::tool_registry::Tool;
use crate::api::types::{FunctionDefinition, ToolDefinition};
use reqwest::Client;

pub struct FetchTool {
    client: Client,
    firecrawl_api_key: String,
}

#[derive(Debug, Deserialize)]
struct FetchResponse {
    data: FetchResponseData,
}

#[derive(Debug, Deserialize)]
struct FetchResponseData {
    markdown: String,
}

impl FetchTool {
    pub fn new(firecrawl_api_key: String) -> Self {
        Self {
            client: Client::new(),
            firecrawl_api_key,
        }
    }
}

#[async_trait]
impl Tool for FetchTool {
    fn name(&self) -> &str {
        "fetch"
    }

    fn definition(&self) -> ToolDefinition {
        // Note: This uses a special type for provider tools
        ToolDefinition {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: "fetch".to_string(),
                description: "Fetch the content of a URL. \
                              Use this to fetch the content of a URL."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "The URL to fetch",
                        },
                    },
                    "required": ["url"],
                }),
            },
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let url = args["url"].as_str().context("Missing 'url' argument")?;

        // Use the Firecrawl API to perform the web search
        let response = self
            .client
            // TODO: experiment with https://www.tavily.com/
            .post(&format!("https://api.firecrawl.dev/v2/scrape"))
            .header(
                "Authorization",
                format!("Bearer {}", self.firecrawl_api_key),
            )
            .header("Content-Type", "application/json")
            .json(&json!({
                "url": url,
                "onlyMainContent": true,
                "maxAge": 172800000,
                "parsers": [
                    "pdf",
                ],
                "formats": [
                    "markdown",
                ],
            }))
            .send()
            .await
            .context("Failed to perform fetch with firecrawl")?
            .json::<FetchResponse>()
            .await
            .context("Failed to parse fetch response")?;

        Ok(response.data.markdown)
    }
}
