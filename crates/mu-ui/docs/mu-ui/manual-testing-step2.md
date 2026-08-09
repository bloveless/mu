# mu-ui — Step 2 Manual Testing Plan

Scope: rollout **step 2 (conversation items)** — submitted prompts and canned
assistant replies as history items (delivered whole, no streaming yet),
sticky-bottom scrollback with all keybindings + mouse wheel, the
native-wrapping experiment, code-fence styling, and the seeded conversation on
first paint. Everything here is exercisable against
`cargo run --example streaming_demo` on a real terminal.

Not in scope (later rollout steps): tool call items, thinking, real streaming,
`Ctrl+O` expand, `Error` items, `y`-yank, scroll-above hint. If you can't
interact with them, that is expected — not a regression.

**Step 1 removals:** the input's Up/Down prompt-recall is gone (step 1's C13).
The scrollback is the prompt history now; `Up`/`Down` with a non-empty input
walk the input's lines only.

## How to run

```sh
cargo build --examples
cargo run --example streaming_demo
```

**Expected first frame** (80×24; widths vary; no side borders anywhere): the
seeded 6-message conversation fills the history pane from the top — first paint
already has scrollback. The input band sits below with `› Type a message…`, and
the footer shows the accumulated seed usage on line 2:

```
~/Projects/github.com/bloveless/mu/crates/mu-ui (bloveless/rust-revived)
↑4.2k ↓2.4k                                          (opencode-go) deepseek-v4-flash
```

The footer's `↑4.2k ↓2.4k` is the six seeded `TurnEnd`s (700 in / 400 out
each) already summed. Line 1 shows your actual cwd + branch.

Notes:

- Use a real terminal (kitty, iTerm2, wezterm, foot, Terminal.app, …).
- The seeded content is delivered whole at launch, so you see it instantly, not
  streamed; streaming pacing arrives in step 5.
- `/quit` is the fastest way out; `Ctrl+C` also quits.

---

## A. First paint and seeded history

- [x] **A1 Seeded conversation.** On launch the history pane is not empty: six
  assistant messages appear immediately (crate manifest with a `toml` code
  fence, event loop, layout, scroll model, footer). The pane is scrolled to the
  bottom (sticky follow), so the last message ("The footer recomputes its
  cwd…") is the last visible row.
- [x] **A2 Code fence styling.** The fenced `[package]`/`name = "mu-ui"` block
  renders on a dim background distinct from the surrounding prose.
- [x] **A3 Footer usage seeded.** Line 2 left reads `↑4.2k ↓2.4k` (sum of the
  six seeded turns). After submitting one prompt it becomes `↑4.9k ↓2.8k`.

## B. Scrolling

With an **empty input**, scroll the history:

- [x] **B1 Line scroll.** `k`/`↑` scroll up one line, `j`/`↓` down one line.
  The first `k`/`↑` also disengages sticky-follow.
- [x] **B2 Page scroll.** `PgUp`/`PgDn` move by a page (the history pane's
  height).
- [x] **B3 Top/bottom.** `g` jumps to the very top (sticky-follow off); `G`
  jumps to the bottom and re-engages sticky-follow.
- [x] **B4 Sticky-follow.** At the bottom, submit any prompt: the history
  window stays pinned to the newest content as the canned reply arrives.
  Scroll up, then submit again: the window *does not* jump back down — you
  stay wherever you scrolled.
- [x] **B5 Mouse wheel.** Wheel up/down anywhere scrolls the history by a few
  lines (with an empty input or not). Wheel-follows the same disengage/re-engage
  rules as the keys.
- [x] **B6 Scroll clamp at top.** Hold `k` until the top: the first message's
  first line is the top row and scrolling stops there (no crash, no wrap).

## C. Input-vs-history key split

- [x] **C1 Empty input scrolls.** With nothing typed, `j`/`k`/`↑`/`↓`/`g`/`G`
  scroll history and never insert text.
- [x] **C2 Non-empty input types.** Type `abc`: `j`/`k` insert the letters,
  `↑`/`↓` walk the input's lines (or do nothing on the first/last), and the
  history does not move.
- [x] **C3 Clear-and-scroll.** Clear the input (Backspace), then `k` scrolls
  history again.

## D. Submitting and canned replies

- [x] **D1 Prompt appears + reply.** Type `what does layout do`, Enter. The
  prompt appears at the bottom of the history pane (sticky-follow keeps it in
  view), the input clears, and a canned reply ("Canned reply to
  \"what does layout do\"…") appears beneath it.
- [x] **D2 Prompt is edge-to-edge.** The submitted prompt has no leading glyph,
  border, or padding — just the text, full width.
- [x] **D3 `/quit` still works.** `/quit` exits cleanly (repeat of step 1's
  A2). `/frobnicate` is still submitted as a prompt, not a command.
- [x] **D4 Esc.** Type text, `Esc` with no turn in flight: nothing happens.
  (The demo's turns are delivered whole, so there is effectively no in-flight
  window to interrupt in step 2 — that becomes observable in step 5.)

## E. Resize and re-wrap

- [x] **E1 Resize re-wraps.** Resize wider and narrower: the seeded messages
  re-wrap to the new width (height cache invalidates), the window stays
  coherent, and nothing crashes.
- [x] **E2 Narrow resize.** Shrink below ~20 cols: long lines (the
  `https://…`-style unbroken words, the URL-like tokens in the manifest) hard
  wrap mid-word rather than overflowing; wide CJK/emoji chars wrap as a unit.
- [x] **E3 Text selection.** Option-drag (macOS) across a wrapped message copies
  clean text — no border glyphs or leading spaces. (Soft-wrapped rows still
  copy as separate lines; that's expected in the alternate screen.)

## F. Quick regression pass (30 seconds)

- [x] Launch → confirm seeded scrollback → `k`×5 up (history scrolls) →
  `G` back down → type a line → submit → canned reply appears and the window
  follows → `/quit` exits cleanly.

---

## Native-wrapping experiment — outcome

Passed: `Paragraph::line_count(width)` matches the rendered row count for word
wrapping, hard-wrapped long words, wide CJK, and emoji (verified by unit tests
`resize_rewraps_item_heights` and the `wrap_probe` checks in `render.rs`), so
no custom `wrap.rs` fallback was needed. `line_count` requires the
`unstable-rendered-line-info` ratatui feature, now enabled in `Cargo.toml`.

---

## Golden snapshot tests

`src/snapshot_tests.rs` drives the app exactly as the harness does — pushing
`AgentEvent`s through the real `UiHandle` channel (and typing+Enter for
prompts) — then renders to a fixed-size `TestBackend` and compares the screen
against golden files in `src/snapshots/`. This is the cell-by-cell visual
record of the UI; it's how we iterate on layout/visuals with the reviewable
screenshots.

Scenarios covered: empty UI, a seeded conversation (prompts + replies + code
fence + separators), the same at 40 columns (re-wrap), scrolled-to-top,
a long single message wrapping, and a streamed multi-delta message.

Workflow:

```sh
cargo test                     # snapshot mismatches fail; new ones emit .snap.new
cargo install cargo-insta      # once, for the review TUI
cargo insta review             # interactively accept/reject snapshot changes
```

Or without `cargo-insta`: regenerate and eyeball the diff:

```sh
INSTA_UPDATE=always cargo test # overwrite the golden files
git diff src/snapshots/        # review exactly what the UI change looks like
```

When you want to try a visual change, say so — I make the edit, regenerate the
snapshots, and show you the diff. Colours are deliberately *not* in these
golden files (ratatui's `TestBackend` text view is style-free); color-level
assertions live as unit tests in `render.rs`.
