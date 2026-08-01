use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::agent::tool_registry::Tool;
use crate::api::types::{FunctionDefinition, ToolDefinition};
use reqwest::Client;

pub struct WebSearchTool {
    client: Client,
    firecrawl_api_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WebSearchResponse {
    success: bool,
    data: WebSearchResponseData,
}

#[derive(Debug, Serialize, Deserialize)]
struct WebSearchResponseData {
    web: Vec<WebSearchResult>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WebSearchResult {
    title: Option<String>,
    description: Option<String>,
    markdown: Option<String>,
    url: String,
}

impl WebSearchTool {
    pub fn new(firecrawl_api_key: String) -> Self {
        Self {
            client: Client::new(),
            firecrawl_api_key,
        }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn definition(&self) -> ToolDefinition {
        // Note: This uses a special type for provider tools
        ToolDefinition {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: "web_search".to_string(),
                description: "Search the web for current information. \
                                              Use this for questions about recent events, \
                                              current facts, or anything that might have \
                                              changed after your training data."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "The search query",
                        },
                    },
                    "required": ["query"],
                }),
            },
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let query = args["query"].as_str().context("Missing 'query' argument")?;

        // Use the Firecrawl API to perform the web search
        let response = self
            .client
            // TODO: experiment with https://www.tavily.com/
            .post(&format!("https://api.firecrawl.dev/v2/search"))
            .header(
                "Authorization",
                format!("Bearer {}", self.firecrawl_api_key),
            )
            .header("Content-Type", "application/json")
            .json(&json!({
                "query": query,
                "sources": [
                    "web",
                ],
                "categories": [],
                "limit": 10,
                "scrapeOptions": {
                    "onlyMainContent": true,
                    "parsers": [
                        "pdf",
                    ],
                    "formats": [
                        "markdown",
                    ],
                },
            }))
            .send()
            .await
            .context("Failed to perform web search with firecrawl")?
            .json::<WebSearchResponse>()
            .await
            .context("Failed to parse web search response")?;

        Ok(serde_json::to_string(&response.data)
            .context("Failed to serialize web search response")?)
    }
}
