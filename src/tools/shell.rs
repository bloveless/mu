use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::fs;
use tokio::process::Command;

use crate::agent::tool_registry::Tool;
use crate::api::types::{FunctionDefinition, ToolDefinition};

// --- RunCommand ---------------------------------------------------------

pub struct RunCommandTool;

#[async_trait]
impl Tool for RunCommandTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            tool_type: "function".into(),
            function: FunctionDefinition {
                name: "run_command".to_string(),
                description: "Execute a shell command and return its output. \
                              Use this for system operations, running scripts, \
                              installing packages, etc."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "The shell command to execute.",
                        },
                    },
                    "required": ["command"],
                }),
            },
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let command = args["command"]
            .as_str()
            .context("Missing 'command' argument")?;

        let output = Command::new("sh").arg("-c").arg(command).output().await;

        match output {
            Ok(result) => {
                let stdout = String::from_utf8_lossy(&result.stdout);
                let stderr = String::from_utf8_lossy(&result.stderr);

                let mut response = String::new();

                if !stdout.is_empty() {
                    response.push_str(&stdout);
                }

                if !stderr.is_empty() {
                    if !response.is_empty() {
                        response.push('\n');
                    }
                    response.push_str("STDERR:\n");
                    response.push_str(&stderr);
                }

                if response.is_empty() {
                    response = format!(
                        "Command completed with exit code {}",
                        result.status.code().unwrap_or(-1)
                    );
                }

                Ok(response)
            }
            Err(e) => Ok(format!("Error executing command: {e}")),
        }
    }

    fn requires_approval(&self) -> bool {
        true
    }
}
