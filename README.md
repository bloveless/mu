# mu

`mu` is a terminal AI coding assistant (a "Claude Code" clone) written in Rust.
It drives an OpenAI-compatible chat-completion agent loop — file reading,
str_replace-style editing, bash execution, and HTTP fetch — and renders an
interactive TUI built with `ratatui` + `tui-scrollview`.

## Requirements

- Rust 1.96+ (toolchain pinned in `Cargo.toml` `rust-version`)
- `OPENCODE_API_KEY` — required, the API key for the OpenAI-compatible endpoint
- `OPENCODE_BASE_URL` — optional, defaults to `https://opencode.ai/zen/v1`

## Running

```sh
cargo run --release
```

The release binary is `target/release/mu`.

## Tools

The agent advertises four tools to the model:

| Tool  | Description |
|-------|-------------|
| `read`  | Read a file's contents |
| `edit`  | Exact, unique string replacement. An empty `old_string` creates a new file with `new_string` (refused if the file already exists); otherwise `old_string` must occur exactly once and is replaced by `new_string` |
| `bash`  | Execute a shell command (`kill_on_drop`, cancelled by Esc) |
| `fetch` | HTTP GET with a 30s timeout |

# Suggested Features

The core responsiveness guarantees are in place: the UI loop is fully
decoupled from the agent, channels are unbounded (never blocking on send), and
pressing the Esc key while focused on the input box cancels the in-flight API
call / running tools without blocking typing or focus changes. Agent errors are
surfaced through `eyre` so they can be reported and fixed. The following are
deliberately deferred improvements, focused on render cost and lifecycle:

## Virtualized history rendering

`render_content` currently rebuilds the entire `Vec<Line>` from the full
`history` every frame and wraps every item via `wrap(...)`. Cost is
`O(history × wrap)` per draw, so in a long or bursty session the render path
dominates the loop and typing/cursor latency grows even though input is
structurally never blocked.

The suggested fix is **virtualization**: only render the lines visible in the
current viewport.

1. Compute wrapped lines once per `HistoryItem` and cache them keyed by the
   current terminal width (so a resize invalidates, but a redraw does not
   re-wrap). Append a new item's wrapped lines incrementally when it arrives.
2. Maintain a flat list of cached lines plus a per-item boundary index so a
   given scroll offset can be mapped back to its originating history item.
3. On `render_content`, slice the cached lines to `[offset .. offset +
   viewport_height]` and render only that window. `ScrollbarState` continues to
   track `content_length` / `viewport_content_length` from the cached counts.

This makes each draw `O(viewport)` instead of `O(history)`, removing the long-
session lag at its root while keeping the existing scroll/follow behavior.

## Other deferred improvements

- **Incremental render cache.** Even short of full virtualization, cache the
  wrapped `Vec<Line>` across frames and only append new history items instead
  of rebuilding from scratch each draw. Removes the repeated `wrap()` work.
- **Bounded channel / backpressure.** `AppEvent` is an unbounded channel. The
  batched-event drain keeps the UI from falling behind in practice, but a
  bounded channel with coalescing of high-frequency events (e.g. streamed
  tokens) would give a hard memory ceiling if the agent ever out-produces the
  UI.
- **Agent respawn instead of exit on death.** Today the app exits (with an
  `eyre` report) when the agent task dies. A friendlier behavior is to surface
  the error as a history item and let the user keep the input box, optionally
  respawning the agent task. This requires lifting the conversation `messages`
  out of `openai_stuff` into shared state so it survives a respawn.
- **Per-tool cancellation feedback.** Esc aborts tool tasks and (via
  `kill_on_drop`) their `bash` children, but the UI does not yet show which
  tool was interrupted beyond the existing `ToolCallStart`/`ToolCallOutput`
  markers. A distinct "cancelled" marker would make interrupts clearer.
