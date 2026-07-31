use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::agent::tool_registry::Tool;
use crate::api::types::{FunctionDefinition, ToolDefinition};

pub struct WebSearchTool;

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

    async fn execute(&self, _args: Value) -> Result<String> {
        // Web search is a provider tool - it is executed server-side.
        // This method is never called directly.
        Ok("Web search is handled by the API provider.".into())
    }
}
