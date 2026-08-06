use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::fs;

use crate::{
    agent::tool_registry::Tool,
    api::types::{FunctionDefinition, ToolDefinition},
};

// --- ReadFile -------------------------------------------------------------------

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
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

pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            tool_type: "function".into(),
            function: FunctionDefinition {
                name: "edit_file".into(),
                description: "Write content to a file at the specified path. Or create new files. \
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
                        "old_string": {
                            "type": "string",
                            "description": "The content that is attempting to be replaced. It must match exactly and only once. To create a new file this should be blank.",
                        },
                        "new_string": {
                            "type": "string",
                            "description": "The new content.",
                        },
                    },
                    "required": ["path", "old_string", "new_string"],
                }),
            },
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let file_path = args["file_path"]
            .as_str()
            .context("Missing 'file_path' argument")?;
        let old_string = args["old_string"]
            .as_str()
            .context("Missing 'old_string' argument")?;
        let new_string = args["new_string"]
            .as_str()
            .context("Missing 'new_string' argument")?;

        if old_string.is_empty() {
            // Create-a-new-file path. An empty `old_string` is the
            // convention for "this file doesn't exist yet"; refuse to
            // clobber an existing file so the model can't accidentally
            // blank out content it meant to edit.
            if fs::try_exists(file_path).await? {
                anyhow::bail!(
                    "tool 'edit' cannot create '{file_path}': file already \
                     exists. To edit it, provide a non-empty, unique \
                     `old_string` that matches the current contents \
                     exactly."
                );
            }
            // Create parent directories so a new file can be added in a
            // new subdirectory in a single step, mirroring `write`.
            if let Some(parent) = std::path::Path::new(file_path).parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent).await?;
            }
            fs::write(file_path, new_string).await?;
            Ok(format!("Created {file_path}"))
        } else {
            let content = fs::read_to_string(file_path).await?;
            match content.matches(old_string).count() {
                0 => anyhow::bail!(
                    "tool 'edit': `old_string` was not found in \
                     '{file_path}'. Make sure it matches the file \
                     exactly, including whitespace and indentation."
                ),
                1 => {
                    let updated = content.replacen(old_string, new_string, 1);
                    fs::write(file_path, updated).await?;
                    Ok(format!("Edited {file_path}"))
                }
                n => anyhow::bail!(
                    "tool 'edit': `old_string` appears {n} times in \
                     '{file_path}'; it must be unique. Include more \
                     surrounding context so it matches exactly one \
                     location."
                ),
            }
        }
    }
}

// --- DeleteFile -------------------------------------------------------------------

pub struct DeleteFileTool;

#[async_trait]
impl Tool for DeleteFileTool {
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
