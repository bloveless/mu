use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;

use crate::agent::tool_registry::Tool;
use crate::api::types::{FunctionDefinition, ToolDefinition};

/// A mock tool that returns a fixed response based on input patterns.
pub struct MockTool {
    tool_name: String,
    description: String,
    parameters: Value,
    responses: HashMap<String, String>,
    default_response: String,
}

impl MockTool {
    pub fn new(
        name: &str,
        description: &str,
        parameters: Value,
        responses: HashMap<String, String>,
        default_response: &str,
    ) -> Self {
        Self {
            tool_name: name.into(),
            description: description.into(),
            parameters,
            responses,
            default_response: default_response.into(),
        }
    }
}

#[async_trait]
impl Tool for MockTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            tool_type: "function".into(),
            function: FunctionDefinition {
                name: self.tool_name.clone(),
                description: self.description.clone(),
                parameters: self.parameters.clone(),
            },
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        // Check each response pattern against the args
        let args_str = args.to_string();
        for (pattern, response) in &self.responses {
            if args_str.contains(pattern) {
                return Ok(response.clone());
            }
        }

        Ok(self.default_response.clone())
    }
}

/// Create a mock registry for file tool evals.
pub fn mock_file_registry() -> crate::agent::tool_registry::ToolRegistry {
    let mut registry = crate::agent::tool_registry::ToolRegistry::new();

    let mut list_responses = HashMap::new();
    list_responses.insert(
        ".".into(),
        "[dir] src\n[file] Cargo.toml\n[file] README.md".into(),
    );
    list_responses.insert("src".into(), "[file] main.rs\n[file] lib.rs".into());

    registry.register(Box::new(MockTool::new(
        "list_files",
        "List files in a directory",
        json!({
            "type": "object",
            "properties": {
                "directory": { "type": "string" },
            },
        }),
        list_responses,
        "[file] unknown.txt",
    )));

    let mut read_responses = HashMap::new();
    read_responses.insert(
        "Cargo.toml".into(),
        "[package]\nname = \"agents-v2\"\nversion = \"0.1.0\"".into(),
    );
    read_responses.insert(
        "main.rs".into(),
        "fn main() {\n    println!(\"Hello, world!\");\n}".into(),
    );

    registry.register(Box::new(MockTool::new(
        "read_file",
        "Read file contents",
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
        }),
        read_responses,
        "Error: file not found",
    )));

    registry
}
