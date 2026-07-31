use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use crate::api::types::ToolDefinition;

#[async_trait]
pub trait Tool {
    /// The tool's name (matches the API).
    fn name(&self) -> &str;

    /// The OpenAI tool definition (sent to the API).
    fn definition(&self) -> ToolDefinition;

    /// Execute the tool with the given arguments.
    async fn execute(&self, args: Value) -> Result<String>;

    /// Whether this tool requires human approval before execution.
    /// Override the return type for dangerous tools.
    fn requires_approval(&self) -> bool {
        false
    }
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Get all tool definitions for the API.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    /// Execute the tool by name.
    pub async fn execute(&self, name: &str, args: Value) -> Result<String> {
        match self.tools.get(name) {
            Some(tool) => tool.execute(args).await,
            None => Ok(format!("Unknown tool: {}", name)),
        }
    }

    /// Check if a tool requires approval.
    pub fn requires_approval(&self, name: &str) -> bool {
        self.tools
            .get(name)
            .map(|t| t.requires_approval())
            .unwrap_or(false)
    }
}
