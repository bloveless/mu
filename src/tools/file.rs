use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::fs;

use crate::agent::tool_registry::Tool;
use crate::api::types::{FunctionDefinition, ToolDefinition};

// --- ReadFile -------------------------------------------------------------------

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            tool_type: "function".into(),
            function: FunctionDefinition {
                name: "read_file".into(),
                description: "Read the contents of a file at the specified path. \
                              Use this to examine file contents."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The path to the file to read.",
                        },
                    },
                    "required": ["path"],
                }),
            },
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args["path"].as_str().context("Missing 'path' argument")?;

        match fs::read_to_string(path).await {
            Ok(contents) => Ok(contents),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(format!("Error: File not found: {path}"))
            }
            Err(e) => Ok(format!("Error reading file: {e}")),
        }
    }
}

// --- ListFiles -------------------------------------------------------------------

pub struct ListFilesTool;

#[async_trait]
impl Tool for ListFilesTool {
    fn name(&self) -> &str {
        "list_files"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            tool_type: "function".into(),
            function: FunctionDefinition {
                name: "list_files".into(),
                description: "List all files and directories in the specified \
                              directory path."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "directory": {
                            "type": "string",
                            "description": "The directory path to list the contents of",
                            "default": ".",
                        },
                    },
                }),
            },
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let directory = args["directory"].as_str().unwrap_or(".");

        match fs::read_dir(directory).await {
            Ok(mut entries) => {
                let mut items: Vec<String> = Vec::new();
                while let Some(entry) = entries.next_entry().await? {
                    let file_type = if entry.file_type().await?.is_dir() {
                        "[dir]"
                    } else {
                        "[file]"
                    };
                    let name = entry.file_name().to_string_lossy().to_string();
                    items.push(format!("{file_type} {name}"));
                }
                items.sort();
                if items.is_empty() {
                    Ok(format!("Directory {directory} is empty."))
                } else {
                    Ok(items.join("\n"))
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(format!("Error: Directory not found: {directory}"))
            }
            Err(e) => Ok(format!("Error listing directory: {e}")),
        }
    }
}

// --- WriteFile -------------------------------------------------------------------

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            tool_type: "function".into(),
            function: FunctionDefinition {
                name: "write_file".into(),
                description: "Write content to a file at the specified path. \
                                              Creates parent directories if they don't exist. \
                                              Overwrites the file if it already exists."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The file path to write to",
                        },
                        "content": {
                            "type": "string",
                            "description": "The content to write",
                        },
                    },
                    "required": ["path", "content"],
                }),
            },
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args["path"].as_str().context("Missing 'path' argument")?;
        let content = args["content"]
            .as_str()
            .context("Missing 'content' argument")?;

        // Create parent directories
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .await
                    .context("Failed to create parent directories")?;
            }
        }

        match fs::write(path, content).await {
            Ok(()) => Ok(format!(
                "Successfully wrote {} bytes to {path}",
                content.len()
            )),
            Err(e) => Ok(format!("Error writing file: {e}")),
        }
    }

    fn requires_approval(&self) -> bool {
        true
    }
}

// --- DeleteFile -------------------------------------------------------------------

pub struct DeleteFileTool;

#[async_trait]
impl Tool for DeleteFileTool {
    fn name(&self) -> &str {
        "delete_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            tool_type: "function".into(),
            function: FunctionDefinition {
                name: "delete_file".into(),
                description: "Delete a file at the specified path.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The path of the file to delete"
                        }
                    },
                    "required": ["path"]
                }),
            },
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args["path"].as_str().context("Missing 'path' argument")?;

        match fs::remove_file(path).await {
            Ok(()) => Ok(format!("Successfully deleted {path}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(format!("Error: File not found: {path}"))
            }
            Err(e) => Ok(format!("Error deleting file: {e}")),
        }
    }

    fn requires_approval(&self) -> bool {
        true
    }
}
