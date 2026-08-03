use anyhow::Result;
use async_trait::async_trait;
use std::collections::BTreeMap;

use serde_json::Value;

use crate::api::types::ToolDefinition;

#[async_trait]
pub trait Tool {
    fn name(&self) -> &str;
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, args: Value) -> Result<String>;
    fn requires_approval(&self) -> bool {
        return true;
    }
}

pub struct ToolRegistry {
    tools: BTreeMap<String, Box<dyn Tool + Send + Sync>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool + Send + Sync>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    pub async fn execute(&self, name: &str, args: Value) -> Result<String> {
        match self.tools.get(name) {
            Some(tool) => tool.execute(args).await,
            None => Ok(format!("Unknown tool: {}", name)),
        }
    }

    pub fn requires_approval(&self, name: &str) -> bool {
        self.tools
            .get(name)
            .map(|t| t.requires_approval())
            .unwrap_or(true)
    }
}
