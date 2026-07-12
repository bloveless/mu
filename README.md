[![progress-banner](https://backend.codecrafters.io/progress/claude-code/ce0763b0-7271-44a0-a838-527ebb5567b2)](https://app.codecrafters.io/users/bloveless?r=2qF)

This is a starting point for Rust solutions to the
["Build Your own Claude Code" Challenge](https://codecrafters.io/challenges/claude-code).

Claude Code is an AI coding assistant that uses Large Language Models (LLMs) to
understand code and perform actions through tool calls. In this challenge,
you'll build your own Claude Code from scratch by implementing an LLM-powered
coding assistant.

Along the way you'll learn about HTTP RESTful APIs, OpenAI-compatible tool
calling, agent loop, and how to integrate multiple tools into an AI assistant.

**Note**: If you're viewing this repo on GitHub, head over to
[codecrafters.io](https://codecrafters.io) to try the challenge.

# Passing the first stage

The entry point for your `claude-code` implementation is in `src/main.rs`. Study
and uncomment the relevant code, and submit to pass the first stage:

```sh
codecrafters submit
```

# Stage 2 & beyond

Note: This section is for stages 2 and beyond.

1. Ensure you have `cargo (1.96)` installed locally.
2. Run `./your_program.sh` to run your program, which is implemented in
   `src/main.rs`. This command compiles your Rust project, so it might be slow
   the first time you run it. Subsequent runs will be fast.
3. Run `codecrafters submit` to submit your solution to CodeCrafters. Test
   output will be streamed to your terminal.

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
