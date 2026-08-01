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
    fn name(&self) -> &str {
        "run_command"
    }

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

// --- CodeExecution ---------------------------------------------------------

pub struct CodeExecutionTool;

#[async_trait]
impl Tool for CodeExecutionTool {
    fn name(&self) -> &str {
        "code_execution"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            tool_type: "function".into(),
            function: FunctionDefinition {
                name: "execute_code".into(),
                description: "Execute a code snippet in the specified language. \
                              Supports python, javascript/node, ruby, and bash."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "language": {
                            "type": "string",
                            "description": "The programming language",
                            "enum": ["python", "javascript", "ruby", "bash"]
                        },
                        "code": {
                            "type": "string",
                            "description": "The code to execute"
                        }
                    },
                    "required": ["language", "code"]
                }),
            },
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let language = args["language"]
            .as_str()
            .context("Missing 'language' argument")?;
        let code = args["code"].as_str().context("Missing 'code' argument")?;

        let (cmd, extension) = match language {
            "python" => ("python", "py"),
            "javascript" => ("node", "js"),
            "ruby" => ("ruby", "rb"),
            "bash" => ("bash", "sh"),
            _ => return Ok(format!("Unsupported language: {}", language)),
        };

        // Write to a temp file
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("agent_code.{extension}"));
        fs::write(&temp_file, code)
            .await
            .context("Failed to write temp file")?;

        let output = Command::new(cmd).arg(&temp_file).output().await;

        // Clean up
        let _ = fs::remove_file(&temp_file).await;

        match output {
            Ok(result) => {
                let stdout = String::from_utf8_lossy(&result.stdout);
                let stderr = String::from_utf8_lossy(&result.stderr);

                if result.status.success() {
                    if stdout.is_empty() {
                        Ok("Code executed successfully (no output)".into())
                    } else {
                        Ok(stdout.to_string())
                    }
                } else {
                    Ok(format!(
                        "Error (exit {}):\n{}{}",
                        result.status.code().unwrap_or(-1),
                        stderr,
                        if !stdout.is_empty() {
                            format!("\nStdout:\n{stdout}")
                        } else {
                            String::new()
                        }
                    ))
                }
            }
            Err(e) => Ok(format!("Failed to execute {cmd}: {e}")),
        }
    }

    fn requires_approval(&self) -> bool {
        true
    }
}
