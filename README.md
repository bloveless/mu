# mu2 — Terminal AI Coding Agent

**mu2** is a terminal-based AI coding assistant that connects to OpenAI-compatible LLM APIs (like DeepSeek, OpenCode, etc.) and runs in an iterative agent loop. The AI can read files, edit files, execute shell commands, and fetch web content — all from inside your terminal with colored, streaming output.

It was written to scratch the author's own itch: a lightweight, local-first agent that doesn't require a GUI, a daemon, or a proprietary service.

## Features

- **Agentic tool loop** — the model proposes tool calls, mu2 executes them, feeds the results back, and loops until the task is done (up to 50 iterations per user message).
- **Streaming responses** — see the model's reasoning and replies as they arrive, color-coded for clarity:
  - 🟡 Yellow = model thinking/reasoning
  - 🔵 Blue = assistant response
  - 🔵 Cyan = tool being invoked
  - ⚪ Light grey = tool result
- **Four built-in tools**:
  - `read` — read file contents
  - `edit` — exact-string-replacement edits (with safety checks for uniqueness and missing strings)
  - `bash` — execute arbitrary shell commands (captures stdout/stderr and exit code)
  - `fetch` — download a URL, extract readable content via Mozilla's Readability, and convert it to Markdown
- **Provider auto-discovery** — fetches the latest provider/model catalog from [models.dev](https://models.dev) and caches it locally for 24 hours.
- **ANSI-colored logging** — debug, tool, thinking, and assistant output are all visually distinct.
- **Go 1.26** (modern Go) — leverages `maps`, `slices`, `errors.As`, `context`, and `io.TeeReader` throughout.

## Getting Started

### Prerequisites

- Go 1.26+
- An API key for a supported provider (currently hardcoded to `OPENCODE_API_KEY` from the [OpenCode](https://opencode.ai) provider)

### Build & Run

```bash
git clone https://github.com/bloveless/mu.git
cd mu2
export OPENCODE_API_KEY=sk-your-api-key-here
go run .
```

You'll be dropped into a REPL. Type your prompts and watch the agent work.

### Command-line flags

| Flag | Description |
|------|-------------|
| `-v` | Enable verbose/debug logging to stderr |

## How It Works

1. **Startup**: mu2 fetches the provider catalog from `https://models.dev/api.json`, caches it to `providers.json`, and selects the `opencode` provider with the `deepseek-v4-flash-free` model.
2. **REPL**: Prompts are read from stdin. Each prompt starts a new agent loop.
3. **Agent loop**: For each iteration (up to 50):
   - Send the conversation (system prompt + messages) to the LLM API over SSE.
   - Stream the response to stdout, displaying reasoning content in yellow and assistant content in blue.
   - If the model returns tool calls, execute each one and append the result as a new message.
   - If no tool calls are returned, the loop terminates and waits for the next user prompt.
4. **System prompt**: The `DEFAULT_INSTRUCTIONS.md` file is embedded at build time and used as the system prompt, instructing the AI on how to use the available tools effectively.

## Project Structure

```
.
├── main.go                    # Entry point, REPL loop, streaming iteration
├── tools.go                   # Tool definitions (read, edit, bash, fetch) + registry
├── tools_test.go              # Tests for the bash tool and truncateLines helper
├── DEFAULT_INSTRUCTIONS.md    # System prompt embedded into the binary
├── providers.json             # Cached provider catalog (fetched from models.dev)
├── .golangci.yml              # Linter configuration
├── go.mod / go.sum            # Go module dependencies
├── api/
│   ├── client.go              # HTTP client for OpenAI-compatible chat completions
│   ├── api.go                 # Provider discovery (models.dev fetcher + caching)
│   └── models.go              # All data types: messages, tool calls, streams, providers
└── logging/
    └── logger.go              # Colored terminal logging helpers
```

## Configuration

Currently mu2 is configured entirely via code in `main.go`:

- **Provider**: hardcoded to `"opencode"`
- **Model**: hardcoded to `"deepseek-v4-flash-free"`
- **API key**: read from the environment variable specified by the provider (e.g., `OPENCODE_API_KEY`)
- **Max iterations**: `MaxIterationsPerUserMessage = 50`

Making these configurable (via config file or CLI flags) is on the roadmap.

## Future Ideas & Improvements

### Short-term

- **Config file** (TOML/YAML) for provider, model, and API key selection instead of hardcoded values.
- **Configurable max iterations** — allow users to set a limit per prompt via a flag.
- **Multiple provider support** — let the user pick from any provider in the catalog at runtime.
- **Conversation history** — persist sessions to disk so you can resume later.
- **Streaming improvements** — handle very long lines from some providers (the scanner default buffer may be too small).

### Medium-term

- **Sub‑agent / context manager** — When the conversation or file state grows large, spawn a sub-agent that summarizes or prunes older context so the main agent stays focused and doesn't exceed the model's context window.
- **File diffing & patch application** — replace the exact-string `edit` tool with a proper diff/patch workflow (unified diff → `git apply` or `patch`).
- **Image attachment support** — some providers support image inputs; pass through attachments from the user.
- **Tool result truncation** — the `truncateLines` helper already exists; wire it up for all tool results to keep context usage under control.
- **Multi-turn conversation navigation** — ability to go back, edit, and re-submit previous prompts.

### Longer-term

- **Pluggable tools** — load custom tools from a user-defined directory (e.g., `~/.mu2/tools/*.lua` or WASM plugins).
- **Built-in file explorer** — a TUI file browser (using bubbletea or charm) for interactive file selection before prompting.
- **Git integration** — automatic commit staging, diff generation, and PR description drafting.
- **RAG / vector search** — index the project's codebase and let the tool calls query relevant files semantically.
- **Multi-model orchestration** — route simple requests to a cheap/fast model and complex ones to a frontier model.
- **Collaborative mode** — share agent state with other instances (or a web UI) for pair debugging.
- **Memory & persistent state** — allow the agent to store facts, preferences, and project-wide decisions across sessions.
- **Think‑tank mode** — when stuck, spawn several parallel sub-agents with different prompts and merge the best result.

### Ideas for Sub‑Agents to Help Manage Context

One of the most promising directions for mu2 is using **sub‑agents** to manage context windows effectively:

1. **Summarizer agent** — When the message list approaches the context limit, a lightweight model condenses the conversation into a compact summary that replaces the older messages.
2. **File‑indexer agent** — Before a coding task, a fast model scans the project structure and builds a manifest of relevant files so the main agent doesn't have to explore blindly.
3. **Plan‑then‑code agent** — The main agent first calls a "planner" sub-agent that produces a high-level task list. The main agent then works through the list step by step, keeping only the current step in context.
4. **Critic agent** — After a tool edit, a separate agent reviews the diff for correctness, style issues, and test coverage before the tool loop continues.
5. **Search agent** — A dedicated agent for `grep`/`find`/`ripgrep` that returns only the matching lines, sparing the main agent from filling context with long search output.

## License

MIT (see repository for details).
