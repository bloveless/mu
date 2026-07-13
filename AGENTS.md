# AGENTS.md

## Project Overview

`mu` is a Rust terminal AI coding assistant (a "Claude Code" clone). It drives an
OpenAI-compatible chat-completion agent loop (file reading, str_replace-style editing,
bash execution, and HTTP fetch) and renders an interactive TUI with `ratatui` +
`tui-scrollview`.

Editing doubles as file creation: the `edit` tool creates a new file when given an empty
`old_string` (refused if the file already exists), and otherwise does an exact, unique
string replacement.

## Essential Commands

```sh
# Build (output: target/release/mu)
cargo build --release

# Run
cargo run --release
```

## Build Details

- **Build command**: `cargo build --release`
- **Binary name**: `mu` (derived from the Cargo package name; no `[[bin]]` override)
- **Binary path**: `target/release/mu`
- **Rust version**: 1.96 (pinned in `Cargo.toml` `rust-version`)
- **Edition**: 2024

## Environment Variables

| Variable           | Required | Default                          |
|--------------------|----------|----------------------------------|
| `OPENCODE_API_KEY` | Yes      | (none — exits with an error)     |
| `OPENCODE_BASE_URL`| No       | `https://opencode.ai/zen/v1`     |

## Code Organization

```
src/
├── main.rs                   # Entry point, agent loop, tool definitions & dispatcher
├── ui.rs                      # TUI app: scrollback history, prompt input, rendering
├── theme.rs                   # Centralized `Theme` of `ratatui::Style` helpers
├── wrap.rs                    # Unicode-aware word wrapping for the TUI
└── DEFAULT_INSTRUCTIONS.md    # System prompt, embedded via include_str!()
```

## Architecture

### main.rs Structure

The app runs in **three execution contexts**, each in its own place. The
boundary lives in the source: above the two `thread::spawn` calls nothing is
async and no runtime exists; the runtime is built and driven entirely inside
the agent thread.

1. **`main()`** — a **plain sync `fn`** (no `#[tokio::main]`). Installs
   `color_eyre` + `console_subscriber` (Tokio console), parses the (currently
   empty) `clap` `Args`, sets up the channels + shared `CancellationToken`, then,
   in order:
   - **Spawns the `"crossterm-events"` std thread** — polls `event::poll`/
     `event::read` (blocking calls that must not run on an async task) and forwards
     `Key`/`Resize` events into the same std channel the agent writes to. Shuts
     down by polling the shared token (≤100ms) or by channel disconnect.
   - **Spawns the `"agent-runtime"` std thread** — this thread **owns the tokio
     runtime**: it builds a multi-thread runtime via
     `tokio::runtime::Builder::new_multi_thread().enable_all().build()` and drives
     `run_agent()` with a single `runtime.block_on(...)` *entirely inside the
     thread*. `main` never holds a runtime handle. The only citizen of the runtime
     is the agent (`openai_stuff`) plus the per-batch tool `JoinSet`s it spawns. If
     the runtime fails to build, a `Fatal` event is sent to the UI.
   - **Runs the ratatui UI directly on the `main` thread** —
     `ui::App::new(event_rx, ai_tx).run(&mut terminal)` with `ratatui::init`/
     `restore` bracketing it on `main`, so raw-mode setup/teardown happens on the
     very thread that draws.
   - After `App::run` returns: cancels the token, `join`s the agent thread (its
     `Ok`/`Err`/panic result decides whether to surface a build/init error), and
     joins the crossterm thread.
2. **`run_agent()`** — runs entirely inside the tokio runtime on the agent thread.
   Drives the sole root task `openai_stuff()`; when it returns (on cancel or `ai_rx`
   disconnect), surfaces any error as a `Fatal` event to the UI. There is **no**
   UI-completion coordination (no oneshot) — `main`/the UI is what knows when the
   app is done and cancels the shared token to wind this side down.
3. **Channels (the only boundary crossing):**
   - agent + crossterm → UI is a **std** `mpsc::channel` (the UI blocks on
     `Receiver::recv_timeout`, never `await`).
   - UI → agent is a **tokio** `mpsc::unbounded_channel` so the agent can `select!`
     over its receiver; its `send` is synchronous, so the sync UI on `main` holds
     the sender without joining the runtime.
4. **`AppEvent` / `AIEvent`** — the two halves of the UI↔agent channel. The UI sends
   `AIEvent::UserPrompt(String, CancellationToken)`; the agent streams back
   `AssistantResponse`, `ToolCallStart`, `ToolCallOutput`, `Error`, `TurnEnd`, and
   fatal `Fatal` events.
5. **`openai_stuff()`** — the live agent loop (NOT dead code):
   - Maintains a `Vec<ChatCompletionRequestMessage>` seeded with the
     `DEFAULT_INSTRUCTIONS` system message.
   - Per turn: pushes the user prompt, records a `checkpoint`, then loops up to
     **20 iterations** at `max_completion_tokens(512)`, model
     `deepseek-v4-flash-free`.
   - Tool set advertised to the model: `read`, `edit`, `bash`, `fetch` (each
     declared `strict(true)`).
   - Each turn is scoped to its own `CancellationToken` so the UI's Esc key cancels
     just that turn; on cancel the conversation is rolled back to `checkpoint` so a
     half-finished tool-call sequence can't break the next request.
   - Tool calls run on a per-batch `JoinSet` so `abort_all()` cancels everything
     atomically; `bash` children use `kill_on_drop(true)`, and `fetch` has a 30s
     `reqwest` client timeout.
6. **`call_fn()`** — the tool dispatcher. Returns `Result<String>`; the Ok value (or
   the error's `to_string()`) is sent back as the tool result message.

### Tool semantics

| Tool  | Params                                   | Behavior |
|-------|------------------------------------------|----------|
| `read`  | `file_path`                            | `tokio::fs::read_to_string` |
| `edit`  | `file_path`, `old_string`, `new_string`| Empty `old_string` ⇒ create new file with `new_string` (parent dirs created; refused if file exists). Non-empty ⇒ `old_string` must occur exactly once (`0`→not found, `>1`→not unique); the single occurrence is replaced and the file written. |
| `bash`  | `command`                              | `bash -c`, `kill_on_drop(true)`, no per-command timeout (user cancels with Esc) |
| `fetch` | `url`                                  | `reqwest` GET, 30s timeout, custom `User-Agent` (`HTTP_USER_AGENT`) |

### TUI Pattern (src/ui.rs)

- Uses `ratatui` + `tui-scrollview`'s `ScrollView`/`ScrollViewState` for the
  scrollable conversation history; the input area is a multiline `Paragraph`.
- **`HistoryItem`** enum: `UserPrompt`, `AssistantResponse`, `SystemError`,
  `ToolCallStart`, `ToolCallOutput` — each wraps a `WrappedItem`.
- **`WrappedItem`** caches word-wrapped lines per item and invalidates on width change,
  so only new/resized messages pay the wrapping cost (the main perf sink noted at the
  top of the file). Total history height and input height are likewise cached.
- **`Focus`** enum (`History` vs `Input`) drives key handling.

#### Keyboard controls

| Mode    | Key            | Action |
|---------|----------------|--------|
| Input   | (printable)    | Insert char |
| Input   | `Enter`        | Submit prompt (when non-empty & not working) |
| Input   | `Shift+Enter`  | Insert newline |
| Input   | `Backspace`/`Delete` | Delete char (char-boundary safe) |
| Input   | `←`/`→`/`Home`/`End` | Move cursor |
| Input   | `Esc`          | Cancel in-flight turn |
| Input   | `Ctrl+C`       | Quit |
| Input   | `Ctrl+J`/`Ctrl+K` | Toggle focus to history |
| History | `j`/`↓`, `k`/`↑` | Scroll line down/up |
| History | `Ctrl+u`/`Ctrl+d` | Page up/down |
| History | `Ctrl+J`/`Ctrl+K` | Toggle focus to input |
| History | `q`            | Quit |
| History | `Esc`          | Return focus to input |

### theme.rs & wrap.rs

- `theme::Theme` exposes `pub fn` style helpers (`user_tag`, `agent_text`,
  `tool_badge_running/success/failed`, `system_error`, `border_idle/active`,
  `status_bar`, `popup_*`, `modal_border`, …) so the UI never inlines `Style`s.
- `wrap::wrap(text, width) -> Vec<String>` is a unicode-width-aware word wrapper
  (`wrap` → `wrap_single_line` → `hard_break`) used by `WrappedItem`.

## Key Dependencies

| Crate               | Purpose |
|---------------------|---------|
| `tokio`             | Async runtime (multi-thread), `process`, `JoinSet` |
| `async-openai`      | OpenAI-compatible chat-completion client (BYOT) |
| `ratatui` + `crossterm` | Terminal UI + event handling |
| `tui-scrollview`    | Scrollable history area |
| `clap`              | CLI argument parsing |
| `color-eyre`        | Error handling with colored output |
| `reqwest`           | HTTP client for the `fetch` tool |
| `tokio-util`        | `CancellationToken` |
| `console-subscriber`| Tokio runtime introspection (debug) |
| `unicode-width`     | Display-width-correct wrapping |

## Notable Gotchas

1. **Edition 2024 + Rust 1.96** — `Cargo.toml` uses `edition = "2024"` and
   `let`-chains (e.g. `if let … && …`), which require a recent toolchain.

2. **Hard-coded model & limits** — `openai_stuff()` pins `deepseek-v4-flash-free` and
   `max_completion_tokens(512)`; change both in `src/main.rs` to target a different
   backend.

3. **`main` is sync; the runtime lives in its own thread** — there is **no
   `#[tokio::main]`**, and `main` never holds a runtime handle. `main()` is a plain
   `fn` that runs the ratatui UI directly and spawns two std helper threads: the
   `crossterm-events` reader and the `agent-runtime` thread, which builds a
   multi-thread tokio runtime via
   `tokio::runtime::Builder::new_multi_thread().enable_all().build()` and drives it
   with a single `runtime.block_on(run_agent(...))` *entirely inside that thread*.
   The only `block_on` in the program lives in the agent thread, never on a runtime
   task, so it cannot self-deadlock. If you add async entry points, keep them
   inside the agent thread's `block_on(...)` and never call `block_on` from within a
   runtime task.

4. **`console_subscriber::init()` is always on** — it registers the Tokio console
   listener on every run; ignore or point `console-subscriber` at it only when debugging
   the runtime.
