# mu-ui: Agent Harness UI Crate

- **Status**: In Review
- **Slug**: `mu-ui`

## Summary

`mu-ui` is a self-contained Rust crate that renders an interactive agent-harness
TUI (messages + thinking + tool calls with scrollback, multiline input, thin
status footer) using ratatui. The library owns the terminal and the crossterm
event loop and is driven entirely by a synchronous channel API — the harness
pushes `AgentEvent`s in and reads `UiEvent`s out. A demo under `examples/`
plays a scripted transcript (streaming thinking, streaming prose, tool calls,
errors) so the UI can be exercised without a real model.

## Context and Scope

The crate starts from a blank `mu-ui` package with only `ratatui 0.30`
installed. It is the intended future UI layer of the `mu` agent, but this doc
covers only the standalone crate: the widgetry, the event API, and the demo
example(s). Glue code in `mu` (thread placement, async bridging) is explicitly
out of scope.

## Goals

- Fully synchronous, async-free library; the only boundary is `std::mpsc`
  channels. Where the UI thread lives is a decision for the consumer's glue
  code, not the crate.
- Library owns the terminal and the blocking crossterm event loop
  (`App::run()`-style), exposing a handle the harness feeds events into.
- Alternate-screen viewport with internal scrollback (unbounded history).
- Emulate the real harness faithfully: streaming thinking, streaming message
  text, tool call lifecycle with ids, per-turn usage, errors.
- Minimal visual design in the spirit of pi/codex: no labels, no timestamps,
  no message separators; content runs edge-to-edge with no side borders or
  padding so terminal selections copy clean text.
- Two-line footer: line 1 shows the working directory (home-relative) and
  git branch; line 2 shows model/provider and token usage.
- Experiment with ratatui's native wrapping instead of a custom word
  wrapper (with a documented fallback if it fails).
- `examples/streaming_demo.rs` auto-plays a scripted transcript on launch and
  answers user prompts with canned streamed replies; `Esc` cancels a stream
  mid-token.
- Copy/paste story: mouse capture on (wheel scroll), `y` yanks the last
  assistant message to the system clipboard.

## Non-Goals

- **Parallel tool calls.** `mu` runs tools sequentially; the UI models one
  in-flight tool call at a time. The event types leave room to add this later.
- **Per-item focus/selection in history** (focus ring, per-item expand).
  Phase 2.
- **Custom text-selection logic** with automatic copy-on-select
  (opencode-style). Phase 2 — needed because mouse capture disables native
  terminal selection; see Cross-Cutting Concerns.
- Full markdown rendering, syntax highlighting, timestamps, cost display.
- Inline (non-alternate-screen) viewport and native terminal scrollback.
- Tokio or any async runtime inside the crate.

## Constraints

- ratatui 0.30 with the crossterm backend; edition 2024.
- The library must not hold or require an async runtime, so the blocking
  `event::poll`/`event::read` calls are legal on whatever thread the consumer
  runs it on.
- Terminal init/teardown (raw mode, alternate screen, mouse capture,
  bracketed paste) must happen on the thread that calls `App::run()` and must
  restore on every exit path, including panic.

## Proposed Design

### Crate layout

```
mu-ui/
├── src/
│   ├── lib.rs        # public API: App, UiHandle, AgentEvent, UiEvent, Usage
│   ├── app.rs        # event loop, terminal setup/teardown, dispatch
│   ├── history.rs    # HistoryItem model, streaming state machine, truncation
│   ├── input.rs      # multiline editor (emacs keys, history recall, paste)
│   ├── render.rs     # layout + drawing (history pane, input band, footer)
│   └── theme.rs      # Theme struct, hardcoded dark-terminal defaults
└── examples/
    └── streaming_demo.rs   # scripted transcript + canned replies
```

The placeholder `src/main.rs` is removed; examples are the only binaries
(`cargo run --example streaming_demo`).

### Threading and channel model

The library is a plain blocking view layer. `App::run()` owns terminal setup,
the crossterm poll loop, and rendering, and returns when the UI exits. The
consumer gets two channel ends:

- `UiHandle` (cloneable `std::mpsc::Sender<AgentEvent>`) — the harness (or
  demo driver) pushes agent activity in. `Send`, so it can live on any
  thread.
- `Receiver<UiEvent>` — the harness reads what the user did: submitted
  prompts and interrupts.

Because both ends are `std::mpsc`, the consumer decides threading: run the UI
on the main thread and drive it from a worker, or spawn the UI on its own OS
thread. `mu`'s glue layer will bridge these to tokio; the crate never knows.

### Event vocabulary

```rust
pub enum AgentEvent {
    TurnStart,
    ThinkingStart,
    ThinkingDelta(String),
    ThinkingEnd,
    MessageStart,
    MessageDelta(String),
    MessageEnd,
    ToolCallStart    { id: ToolCallId, name: String },
    ToolCallArgsDelta{ id: ToolCallId, delta: String },
    ToolCallRunning  { id: ToolCallId },
    ToolCallEnd      { id: ToolCallId, result: Result<String, String> },
    TurnEnd          { usage: Usage },
    Error(String),
}

pub struct Usage { pub input_tokens: u64, pub output_tokens: u64 }

pub enum UiEvent {
    UserPrompt(String),
    Interrupt,          // Esc while a turn is in flight
}
```

Tool calls carry a `ToolCallId` so `*Delta`/`Running`/`End` match their
`Start`; with sequential execution at most one call is open at a time, but the
id keeps the protocol honest and leaves room for parallelism later. The lib is
a pure state machine over these events: deltas append to the currently open
history item, `*End` seals it. `Interrupt` is how Esc reaches the harness —
the lib reports it and immediately seals any open streams so the view stays
coherent whether or not the harness sends more events; the harness decides
what cancellation means. In the demo, the driver thread watches for
`Interrupt` and abandons the script.

### History rendering and scrolling

History is a `Vec<HistoryItem>` (`Thinking`, `Message`, `ToolCall`, `Error`).
**Wrapping is delegated to ratatui's native `Paragraph` wrapping** (an
experiment): each item's height comes from `Paragraph::line_count(width)`,
cached per item and invalidated only on width change, and partially visible
items are positioned with `Paragraph::scroll`. The flattened-line offset
model still drives scroll position — only the visible window is drawn each
frame and no large offscreen buffer is needed. If the experiment fails
(unicode-width bugs, `line_count` cost), a custom `wrap.rs` returns as the
fallback. All content renders edge-to-edge — no side borders, padding, or
indentation — so a terminal selection across a message captures clean text
without border glyphs or leading spaces.

Scrolling: sticky-bottom auto-follow (any user scroll up disengages follow;
scrolling to the bottom re-engages), `j`/`k` + arrows by line, `PgUp`/`PgDn`
by page, `g`/`G` top/bottom, mouse wheel. While scrolled up, the input's top
border shows a `— N lines above —` hint.

Truncation and the global expand toggle (`Ctrl+O`):

- **Thinking**: while streaming, and after completion, shows the *last* 4
  lines (a live tail). Collapsed items end with a `… ctrl+o to expand` hint.
- **Tool call**: header line (status glyph + tool name, e.g. `✓ Bash`,
  `✗ Bash`), then the *full* command/args (never truncated), then the *last*
  4 lines of output with a `… N more lines` hint — tailing keeps the live,
  most-relevant end of the output visible while a call runs.
- `Ctrl+O` toggles every collapsed item in history between truncated and
  full. There is no per-item expansion in v1.

Markdown: minimal. Code fences get a distinct background/dim block; all other
text renders plainly. No markdown parser dependency.

### Input band

Full-width multiline editor separated from history by horizontal rules —
borders on top and bottom only, no side borders. Grows from 1 to 4 lines with
content, then scrolls internally. `› ` prompt glyph with dim placeholder text.

- `Enter` submits (when non-empty); `Shift+Enter` inserts a newline.
- Bracketed paste: pasted text (including newlines) is inserted literally and
  never submits early.
- Emacs-ish editing: arrows, `Home`/`End`, `Ctrl+A`/`Ctrl+E` (line start/end),
  `Ctrl+K` (kill to end), `Ctrl+U` (kill line), `Ctrl+W` (kill word).
- `Up`/`Down` recall previously submitted prompts.
- Slash commands: input beginning with `/` is matched against a small command
  registry before submission. v1 ships `/quit` only (the lib intercepts it and
  shuts down cleanly); the registry is the seam for future commands.
- `y` copies the last assistant *message text* (no thinking, no tool output)
  to the clipboard via `arboard`.
- `Esc` during an in-flight turn emits `UiEvent::Interrupt`.

### Footer

Two thin lines at the very bottom.

- **Line 1**: current working directory relative to the user's home
  (`~/Projects/mu`), plus the current git branch when inside a repo (e.g.
  `~/Projects/mu  main`). The lib computes both itself — cwd from
  `std::env::current_dir`, branch by walking up to `.git/HEAD` — cached and
  refreshed on each `TurnStart`/prompt submit rather than per frame; the
  harness may override either string at init.
- **Line 2**: left, the model/provider string set at init (e.g.
  `deepseek-v4-flash-free · opencode`); right, cumulative session token
  counts from `TurnEnd` usage (e.g. `↑12.4k ↓3.1k`). No cost display.

### Theme

A `Theme` struct of `ratatui::Style` values with hardcoded dark-terminal
defaults (dim/italic thinking, green/red tool status, muted borders and
footer). Hardcoded now; the struct is the configuration seam later.

### Demo example

`examples/streaming_demo.rs` spawns a driver thread holding the `UiHandle`
and immediately auto-plays a scripted transcript with realistic pacing:
thinking that tails live, streamed prose, several tool calls (including one
with long truncated output and one that fails), a fenced code block, and
usage updates — enough content that scrollback exists on first paint. After
the transcript, every user prompt gets a canned streamed reply; `Esc`
interrupts any stream mid-token via `UiEvent::Interrupt`; `/quit` exits. More
examples can be added alongside it later (`cargo run --example <name>`).

## Alternatives Considered

- **Inline viewport (pi-style) with native terminal scrollback.** Loses: the
  chosen internal-scrollback model gives predictable layout, a stable input
  band, and scroll keybindings that don't fight the terminal; per the project
  direction, native scrollback is explicitly not required. The cost is copy/
  paste friction, mitigated by mouse-capture + `y` now and custom selection
  in phase 2.
- **Widget library the consumer embeds in its own ratatui loop.** Loses: two
  owners of the terminal and two event loops to reconcile; the consumer
  wanted to feed events, not compose widgets. An owned loop behind a channel
  API is the smaller contract.
- **`tui-scrollview` offscreen buffer (as `mu` uses today).** Loses: unbounded
  history forces an ever-growing fixed-size buffer and re-rendering offscreen
  content every frame; the flattened-line offset model only ever lays out the
  visible window and keeps only per-item height caches.
- **Markdown parser (termimad et al.).** Loses: dependency weight and
  rendering edge cases for a demo whose goal is harness chrome, not prose
  fidelity. Code-fence styling covers the visible need.
- **OSC52 clipboard instead of `arboard`.** Loses for v1: escape-sequence
  support varies across terminals/tmux; `arboard` is reliable on the target
  (macOS). OSC52 can be added as a fallback later.

## Tradeoffs

- Owning the event loop makes the crate trivial to drive but means consumers
  with an existing ratatui app can't embed it — accepted, since the only
  planned consumer (`mu`) wants the whole loop owned.
- Alternate screen + mouse capture breaks native text selection (Option-drag
  on macOS still works). Accepted and explicitly deferred to the phase-2
  custom-selection feature.
- Unbounded history is unbounded memory; fine for a demo and for `mu`
  session lengths, but no pruning story exists yet.
- A global `Ctrl+O` expand-all is blunt (expanding one huge tool output
  expands all of them); per-item expansion is the known better UX and is
  deliberately postponed.

## Cross-Cutting Concerns

- **Copy/paste roadmap (phase 2)**: custom selection logic that re-enables
  text selection inside the alternate screen and automatically copies the
  selection to the clipboard, opencode-style. The v1 mitigations are
  Option-drag passthrough and `y`-to-yank. The edge-to-edge layout keeps
  those selections free of border glyphs; note that soft-wrapped lines still
  copy as separate rows — terminals can't see logical lines in the alternate
  screen.
- **Reliability**: all terminal state (raw mode, alt screen, mouse capture,
  bracketed paste) is restored on drop/exit paths; a panic must not leave the
  user's terminal broken.
- **Performance**: per-item wrap caching + visible-window-only rendering keep
  streaming deltas O(visible); unbounded history growth is the accepted
  tradeoff above.
- **Protocol stability**: the `AgentEvent`/`UiEvent` enums are the real
  product — `mu` will code against them. New variants are expected; existing
  ones should evolve additively.

## Rollout and Migration

Small, individually reviewable steps — every step leaves
`cargo run --example streaming_demo` runnable and interactable:

1. **Skeleton**: `lib.rs` + module files, `AgentEvent`/`UiEvent`/`Usage`
   types, channel API, `App::run()` with terminal setup/teardown (alt screen,
   mouse capture, bracketed paste). Empty history, static layout: input band
   and two-line footer render, typing and `Enter` work, `/quit` exits.
   *Interact with:* editing keys, footer cwd/branch, clean terminal restore.
2. **Conversation items**: submitted prompts and canned assistant replies
   appear as history items (delivered whole, no streaming yet). Sticky-bottom
   scroll with all keybindings + mouse wheel; the native-wrapping experiment
   lands here (`line_count`-driven heights). The demo seeds a long canned
   conversation at startup so scrollback exists on first paint.
   *Interact with:* scrolling a long transcript, resize re-wrap, edge-to-edge
   text selection.
3. **Tool call items**: header + full command + tail-4 output truncation,
   status glyphs, `Ctrl+O` global expand, `Error` items. The demo transcript
   gains tool calls (a failing one, a long-output one), still delivered
   instantly. *Interact with:* expand/collapse over a long mixed history.
4. **Thinking items**: tail-4 thinking blocks with expand. The demo
   transcript gains thinking. *Interact with:* truncated vs expanded
   thinking.
5. **Streaming**: the demo driver paces `*Delta` events over time — live
   thinking tail, token-by-token prose, streaming tool output; usage ticks on
   `TurnEnd`; `Esc` issues `UiEvent::Interrupt` and the driver abandons the
   script. *Interact with:* watching a full turn stream, cancelling
   mid-stream.
6. **Copy affordances**: `y` yanks the last assistant message via `arboard`;
   scroll-above hint in the input border. *Interact with:* pasting the yank
   elsewhere, bracketed paste of multiline text into the input.
7. **Polish**: theme/glyph tuning against real terminals, footer formatting,
   performance pass on long histories.

Later, `mu` replaces its `ui.rs` by bridging `AppEvent`/`AIEvent` to
`AgentEvent`/`UiEvent` in its glue layer (out of scope here).

## Open Questions

- Exact thinking/tool glyphs and colors — will settle during implementation
  against real terminals; the `Theme` struct makes them trivial to change.
- Whether token counts in the footer should also show context-window
  percentage once `mu` knows its limits — additive later.
- Which additional examples are worth adding (e.g. a stress example that
  floods history to validate scroll performance).
- Whether ratatui's native wrapping holds up (unicode width, `line_count`
  cost with caching) or we fall back to a custom `wrap.rs` — decided during
  rollout step 2.

## Decision

Proceed as designed: synchronous channel-driven crate, alternate screen,
internal scrollback with global expand toggle, minimal chrome, scripted demo
example. Reviewer: @brennon. Implementation does not begin until this doc is
approved.
