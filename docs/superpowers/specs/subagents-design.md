# Subagents Design Document

## Overview

Implement subagent functionality where the parent agent can spawn multiple subagents that run in parallel. Subagents cannot spawn their own subagents (no recursion). Communication between parent and subagent uses structured output.

## Core Design Decisions

### Subagent as a Tool

The parent agent spawns subagents via a tool called `delegate`. This fits naturally into the existing tool execution model:

- Tool calls run in parallel via `JoinSet`
- Parent agent waits for tool completion (naturally pauses)
- Tool result is returned to the parent agent
- Existing tool infrastructure handles cancellation and error propagation

### Configuration

Model selection is config-driven via CLI flags:

```bash
mu --model claude-3-opus --subagent-model claude-3-haiku
```

Both parent and subagent use the same API key and base URL, but can use different models.

### No Recursion

Subagents cannot spawn their own subagents. This keeps the system simple and prevents unbounded agent hierarchies.

## Architecture

### Refactored Agent Loop

Extract the agent loop into a reusable function `run_agent_loop()` that both parent and subagent can call:

```rust
pub async fn run_agent_loop(
    config: &AgentConfig,
    ctx: &ToolContext,
    messages: &mut Vec<ChatCompletionRequestMessage>,
) -> Result<AgentResult>
```

The loop takes a mutable reference to the message history and returns the final result. The parent agent's messages persist across turns (maintained in the outer loop), while each subagent gets a fresh messages vec. This allows:

- Parent agent to maintain conversation history across turns
- Subagent to have its own isolated conversation context
- Same loop logic for both parent and subagent

The loop checks `ctx.turn_token.is_cancelled()` at each iteration and breaks early if cancelled, returning partial results.

### AgentConfig

Bundles all agent-specific configuration:

```rust
pub struct AgentConfig {
    pub model: String,
    pub system_prompt: String,
    pub tools: Vec<ChatCompletionTool>,
    pub max_tokens: u32,
    pub temperature: f32,
    pub max_iterations: u32,
}
```

### ToolContext

Bundles shared runtime context (owned fields, no lifetimes):

```rust
pub struct ToolContext {
    pub client: Client<OpenAIConfig>,  // async-openai Client for now
    pub event_tx: std::sync::mpsc::Sender<AppEvent>,
    pub app_token: CancellationToken,
    pub turn_token: CancellationToken,
    pub subagent_model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub max_iterations: u32,
}
```

Currently uses `async-openai`'s `Client<OpenAIConfig>`. When migrating to direct `reqwest` calls (future work), this becomes `reqwest::Client` plus `api_key` and `base_url` fields. All other fields remain the same.

## Tool Definitions

### Shared Tool Builder

Extract tool definitions into a separate module `tools.rs`:

```rust
pub fn base_tools() -> Vec<ChatCompletionTool> {
    vec![read_tool(), edit_tool(), bash_tool(), fetch_tool()]
}

pub fn parent_tools() -> Vec<ChatCompletionTool> {
    let mut tools = base_tools();
    tools.push(delegate_tool());
    tools
}

pub fn subagent_tools() -> Vec<ChatCompletionTool> {
    base_tools()
}
```

The parent agent gets all tools including `delegate`. Subagents get only the base tools (no `delegate`).

### Delegate Tool

The `delegate` tool allows the parent to spawn a subagent:

```json
{
  "name": "delegate",
  "description": "Spawn a subagent to work on a task independently",
  "parameters": {
    "type": "object",
    "properties": {
      "task": {
        "type": "string",
        "description": "Detailed description of the task for the subagent"
      }
    },
    "required": ["task"]
  }
}
```

## Subagent Execution

### Handler

The `delegate` tool is handled in `call_fn()`:

```rust
"delegate" => {
    let task = args.task.clone();
    
    // Emit SubagentStart event
    ctx.event_tx.send(AppEvent::SubagentStart { task: task.clone() });
    
    // Create subagent config using values from ToolContext
    let sub_config = AgentConfig {
        model: ctx.subagent_model.clone(),
        system_prompt: include_str!("SUBAGENT_INSTRUCTIONS.md").to_string(),
        tools: subagent_tools(),
        max_tokens: ctx.max_tokens,
        temperature: ctx.temperature,
        max_iterations: ctx.max_iterations,
    };
    
    // Run subagent loop with fresh messages vec
    let mut sub_messages = Vec::new();
    sub_messages.push(ChatCompletionRequestMessage::User(
        ChatCompletionRequestUserMessageArgs::default()
            .content(task)
            .build()?
            .into()
    ));
    
    let result = run_agent_loop(&sub_config, ctx, &mut sub_messages).await;
    
    // Extract metadata from sub_messages
    let metadata = extract_metadata(&sub_messages);
    
    // Build structured result
    let sub_result = SubagentResult {
        status: if result.is_ok() { "success" } else { "error" },
        summary: extract_summary(&sub_messages),
        files_changed: metadata.files_changed,
        tool_calls: metadata.tool_calls,
        errors: metadata.errors,
    };
    
    // Emit SubagentEnd event
    ctx.event_tx.send(AppEvent::SubagentEnd { result: sub_result.clone() });
    
    // Return structured JSON to parent
    Ok(serde_json::to_string(&sub_result)?)
}
```

### Metadata Tracking

Track subagent activity by analyzing the conversation messages after the subagent completes. This post-hoc approach is simpler than tracking during execution:

```rust
struct SubagentMetadata {
    files_changed: Vec<String>,
    tool_calls: u32,
    errors: Vec<String>,
}

fn extract_metadata(messages: &[ChatCompletionRequestMessage]) -> SubagentMetadata {
    let mut metadata = SubagentMetadata::default();
    
    for msg in messages {
        if let ChatCompletionRequestMessage::Assistant(assistant_msg) = msg {
            if let Some(tool_calls) = &assistant_msg.tool_calls {
                for tool_call in tool_calls {
                    metadata.tool_calls += 1;
                    
                    // Track file changes from edit tool
                    if tool_call.function.name == "edit" {
                        if let Ok(args) = serde_json::from_str::<serde_json::Value>(&tool_call.function.arguments) {
                            if let Some(path) = args["file_path"].as_str() {
                                metadata.files_changed.push(path.to_string());
                            }
                        }
                    }
                }
            }
        }
        
        // Track errors from tool results
        if let ChatCompletionRequestMessage::Tool(tool_msg) = msg {
            if let Some(content) = &tool_msg.content {
                if content.starts_with("Error") || content.contains("failed") {
                    metadata.errors.push(content.clone());
                }
            }
        }
    }
    
    metadata
}
```

### Structured Output Format

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct SubagentResult {
    pub status: SubagentStatus,
    pub summary: String,          // Final assistant message or error message
    pub files_changed: Vec<String>,
    pub tool_calls: u32,
    pub errors: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Completed,   // Subagent finished successfully
    Failed,      // Subagent encountered an error
    Cancelled,   // Subagent was cancelled by user (Esc key)
}
```

This structured JSON is returned to the parent agent as the tool result, allowing it to reason about what the subagent accomplished.

## UI Changes

### AppEvent Extensions

Add new event variants for subagent lifecycle:

```rust
pub enum AppEvent {
    // ... existing variants ...
    
    SubagentStart {
        task: String,
    },
    
    SubagentEnd {
        result: SubagentResult,
    },
}
```

### UI Indicators

Extend the `App` struct to track subagent state:

```rust
pub struct App {
    // ... existing fields ...
    
    /// Number of active subagents
    subagent_count: u32,
    
    /// History of completed subagent results
    subagent_results: Vec<SubagentResult>,
}
```

Update the input border to show both parent and subagent status:

```rust
// When parent is working
let border = if app.subagent_count > 0 {
    format!("● working... ◐ {} subagent{} active", 
            app.subagent_count,
            if app.subagent_count > 1 { "s" } else { "" })
} else {
    "● working...".to_string()
};
```

### Event Handling

Handle `SubagentStart` and `SubagentEnd` events:

```rust
AppEvent::SubagentStart { .. } => {
    app.subagent_count += 1;
    // Don't add to history yet (intermediate tool calls hidden)
}

AppEvent::SubagentEnd { result } => {
    app.subagent_count -= 1;
    app.subagent_results.push(result.clone());
    
    // Add to history as a distinct item
    app.history.push(HistoryItem::SubagentResult { 
        result,
        wrapped: WrappedItem::new(format_subagent_result(&result), width),
    });
}
```

### History Item

Add new history item variant:

```rust
pub enum HistoryItem {
    // ... existing variants ...
    
    SubagentResult {
        result: SubagentResult,
        wrapped: WrappedItem,
    },
}
```

Render with distinct styling:

```rust
HistoryItem::SubagentResult { result, wrapped } => {
    let tag = "▌ SUBAGENT ";
    let style = match result.status.as_str() {
        "success" => Theme::tool_badge_success(),
        "error" => Theme::tool_badge_failed(),
        _ => Theme::tool_badge_running(),
    };
    
    // Render tag
    scroll_view.render_widget(
        Paragraph::new(tag).style(style),
        Rect::new(x, y, tag.len() as u16, 1),
    );
    
    // Render wrapped result
    render_wrapped_item(scroll_view, wrapped, x, y + 1, Theme::agent_text());
    
    y += 1 + wrapped.wrapped_lines().len() as u16;
}
```

### Hidden Tool Calls

When `app.subagent_count > 0`, suppress `ToolCallStart` and `ToolCallOutput` events from being added to history:

```rust
AppEvent::ToolCallStart { .. } | AppEvent::ToolCallOutput { .. } => {
    if app.subagent_count == 0 {
        app.history.push(...);
    }
    // Otherwise, silently ignore
}
```

This keeps the UI clean while the subagent is working. The final `SubagentResult` shows what was accomplished.

## System Prompts

### Parent Agent

Continue using `DEFAULT_INSTRUCTIONS.md` for the parent agent.

### Subagent

Create a new `SUBAGENT_INSTRUCTIONS.md` with specialized instructions:

```markdown
You are a subagent working on a specific task. Focus on completing the task efficiently and effectively.

## Guidelines

- Complete the assigned task
- Use tools as needed to read, edit, and test code
- Provide a clear summary of what you accomplished
- Report any errors or issues encountered

## Important

- You cannot spawn other subagents
- Focus only on the task described below
- Be thorough but efficient
```

## Error Handling & Cancellation

### Cancellation Flow

Subagents respect both cancellation tokens:

- **App-level token** (`app_token`): Cancels entire runtime (Ctrl+C)
- **Turn-level token** (`turn_token`): Cancels current turn (Esc key)

When cancelled, the subagent loop breaks early and returns what it has accomplished so far. The `SubagentResult` includes partial metadata (files changed, tool calls made before cancellation).

### Error Propagation

Subagent errors are contained:

- If subagent fails, `SubagentResult.status = "error"`
- Error message is included in `summary` and `errors` fields
- Parent agent receives the result and can decide how to proceed
- Parent agent is not killed by subagent errors

### Parallel Subagents

Multiple `delegate` calls in a single turn run in parallel via the parent's `JoinSet`:

- Each subagent gets its own isolated conversation context
- Each subagent tracks its own metadata independently
- All subagents are cancelled together if the parent turn is cancelled
- Results are returned as they complete (order doesn't matter)

## Future Enhancements

This design prepares for future enhancements without implementing them now:

1. **Streaming progress**: Could emit intermediate events from subagent (hidden for now)
2. **Dynamic model selection**: Could add `model` parameter to `delegate` tool
3. **Custom tool sets**: Could allow subagents to use different tool combinations
4. **Recursive delegation**: Could remove the restriction on subagent spawning
5. **Progress visibility**: Could show subagent tool calls in collapsible UI sections

## Implementation Notes

### File Organization

- `src/agent.rs` - AgentConfig, run_agent_loop(), AgentResult
- `src/tools.rs` - Tool definitions, call_fn(), ToolContext
- `src/subagent.rs` - SubagentResult, delegate handler, metadata extraction
- `src/main.rs` - Entry point, CLI args, thread setup
- `src/ui.rs` - UI rendering, event handling
- `src/theme.rs` - Styling (extend with subagent styles)
- `SUBAGENT_INSTRUCTIONS.md` - Subagent system prompt

### Testing Strategy

- Unit tests for `extract_metadata()` function
- Unit tests for `run_agent_loop()` with mocked responses
- Integration test: parent spawns subagent, verify structured output
- Integration test: parallel subagents, verify all complete
- Integration test: cancellation flow, verify partial metadata

## Migration Path

The current design uses `async-openai`'s `Client<OpenAIConfig>`. When migrating to direct `reqwest` calls:

1. Change `ToolContext.client` from `Client<OpenAIConfig>` to `reqwest::Client`
2. Add `api_key: String` and `base_url: String` fields to `ToolContext`
3. `reqwest::Client` is cheaply cloneable (Arc-backed), so owned fields work perfectly
4. `run_agent_loop()` signature remains the same
5. Only the HTTP request construction inside the loop changes (build OpenAI-compatible JSON requests)
6. All tool handling and metadata tracking remains unchanged
7. No changes to `AgentConfig`, `SubagentResult`, or UI code
