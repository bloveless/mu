# AGENTS.md

## Project Overview

This is a Rust implementation of an AI coding assistant (Claude Code) for the [Codecrafters challenge](https://codecrafters.io/challenges/claude-code). It uses OpenAI-compatible tool calling to implement an agent loop with file reading, writing, and bash execution capabilities.

## Essential Commands

```sh
# Run locally (compiles then executes)
./your_program.sh

# Submit to Codecrafters
codecrafters submit
```

## Build Details

- **Build command**: `cargo build --release`
- **Target directory**: `/tmp/codecrafters-build-claude-code-rust`
- **Binary name**: `codecrafters-claude-code`
- **Rust version**: 1.96 (specified in Cargo.toml and codecrafters.yml)

## Environment Variables

| Variable | Required | Default |
|----------|----------|---------|
| `OPENCODE_API_KEY` | Yes | (none - exits with error) |
| `OPENCODE_BASE_URL` | No | `https://opencode.ai/zen/v1` |

## Code Organization

```
src/
├── main.rs    # Entry point: CLI parsing, API client, agent loop
└── ui.rs      # TUI application (scrollview demo adapted from tui-scrollview)
```

## Architecture

### main.rs Structure
1. CLI argument parsing via `clap` (Args struct - currently unused)
2. OpenAI client initialization with custom base URL and API key
3. TUI initialization via `ratatui::run()`
4. **Dead code**: Lines 40-193 contain an agent loop using tool calling that is unreachable due to `return Ok(());` on line 38

### Agent Loop Pattern (dead code)
The commented-out agent loop demonstrates:
- OpenAI-compatible tool calling with `async-openai` using BYOT (Bring Your Own Tools)
- Tool types: `Read`, `Write`, `Bash`
- Async file operations via `tokio::fs`
- Message history accumulation for context

### TUI Pattern (src/ui.rs)
- Uses `ratatui` for terminal UI rendering
- `ScrollView` for scrollable content areas
- Keyboard controls: `j/k` (up/down), `g/G` (top/bottom), `Ctrl+d/u` (page), `q/Esc` (quit)

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `tokio` | Async runtime with multi-threaded executor |
| `async-openai` | OpenAI-compatible API client with BYOT support |
| `ratatui` | Terminal UI framework |
| `crossterm` | Cross-platform terminal handling |
| `clap` | CLI argument parsing |
| `color-eyre` | Error handling with colored output |

## Notable Gotchas

1. **Dead code in main.rs:38** - `return Ok(());` before the agent loop makes lines 40-193 unreachable. The actual application entry is `ratatui::run(|terminal| App::new().run(terminal))?;`

2. **Edition 2024** - Cargo.toml uses `edition = "2024"` which is a very recent Rust edition

3. **Unstable const generics** - `src/ui.rs:50` uses `Size::new(s, SCROLLVIEW_HEIGHT)` where `SCROLLVIEW_WIDTH: u16 = 100` - the `s` is a const generic syntax that requires Rust 1.96

4. **Build output location** - Binary is built to `/tmp/codecrafters-build-claude-code-rust/release/`, not the standard `target/release/`

5. **codecrafters.yml controls remote build** - This file controls the Rust version used on Codecrafters servers, not local development
