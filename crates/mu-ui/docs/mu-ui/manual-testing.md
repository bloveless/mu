# mu-ui — Step 1 Manual Testing Plan

Scope: rollout **step 1 (skeleton)** — channel API, terminal lifecycle, input
editing, footer, `/quit`. Everything here is exercisable against
`cargo run --example streaming_demo` on a real terminal.

Not in scope (later rollout steps): scrollback, conversation/tool/thinking
history items, streaming, `y`-yank, `Ctrl+O` expand, scroll-above hint.
If you can't interact with them, that is expected — not a regression.

## How to run

```sh
cargo build --examples
cargo run --example streaming_demo
```

**Expected first frame** (widths vary; no side borders anywhere):

```
──────────────────────────────────────────────────────────────
› Type a message…
──────────────────────────────────────────────────────────────
~/Projects/github.com/bloveless/mu/crates/mu-ui (bloveless/rust-revived)
↑0 ↓0                                              (opencode-go) deepseek-v4-flash
```

Notes:

- Use a real terminal (kitty, iTerm2, wezterm, foot, Terminal.app, …), not a
  pager or `script`. **Shift+Enter** and **bracketed paste** depend on
  terminal support; kitty/iTerm2/wezterm/foot report Shift+Enter distinctly,
  Terminal.app does not (there it degrades to Enter — expected).
- Footer line 1 shows the crate dir home-relative plus the git branch;
  substitute your actual branch name in expectations below.
- Tests in sections B6, D4–D6 need the test driver in Appendix A. Everything
  else works with the stock demo.
- `/quit` is the fastest way out; `Ctrl+C` also quits.

---

## A. Launch and terminal lifecycle

- [x] **A1 Launch.** `cargo run --example streaming_demo`. The screen switches
  to the alternate screen and the first frame above renders: two horizontal
  rules around the input band, `›` glyph, dim placeholder, two footer lines.
  The area above the input band (history pane) is blank.
- [x] **A2 Clean `/quit` exit.** Type `/quit`, Enter. The app exits, the shell
  prompt returns, and the terminal is fully restored: typing echoes, `Ctrl+C`
  interrupts the shell again, and the shell's pre-launch screen contents
  (scrollback) are back. No garbage escape sequences left on screen.
- [x] **A3 Clean `Ctrl+C` exit.** Launch, press `Ctrl+C`. Same restore as A2
  (raw mode disables SIGINT, so this exercises the key-event quit path, not
  a signal).
- [x] **A4 No-turn `Esc`.** Launch, type `hello`, press `Esc`. Nothing happens:
  text stays, no prompt change. `Esc` only acts during an in-flight turn.
- [x] **A5 Non-tty launch (graceful failure).**
  `cargo run --example streaming_demo >/tmp/out 2>&1 </dev/null` prints an
  error and exits promptly (exit code 1). It must not hang, and must not
  leave the calling shell in raw mode.
- [ ] **A6 Panic restore (optional, needs Appendix B).** With the injection
  patch, launch and wait ~1 s: the terminal restores to the shell, the panic
  message prints, the process exits non-zero, and the shell is fully usable
  afterwards. Revert the patch after the test.

## B. Footer

- [x] **B1 Home-relative cwd.** From the crate dir the footer shows
  `~/Projects/github.com/bloveless/mu/crates/mu-ui` (matches `pwd` with
  `$HOME` → `~`). From `$HOME` it shows just `~`; from `/tmp` it shows the
  absolute `/tmp` (not home-relative).
- [x] **B2 Branch.** The branch, parenthesized after the cwd on footer
  line 1, matches `git branch --show-current`. From a detached HEAD it
  shows an 8-char hash instead.
- [x] **B3 Model/provider.** Footer line 2 right shows
  `(opencode-go) deepseek-v4-flash`.
- [ ] **B4 Usage.** Initially `↑0 ↓0`, left-aligned on footer line 2. With
  the test driver
  (Appendix A), after each simulated `TurnEnd` the totals accumulate:
  `↑12.4k ↓3.1k` after one prompt, `↑24.8k ↓6.2k` after two (k-formatting
  kicks in at ≥1000).
- [ ] **B5 Context refresh on submit.** While the demo runs, switch branch in
  a second terminal (`git checkout -` or `git switch some-other-branch`),
  then submit any prompt. Footer line 1 updates to the new branch within a
  frame. (Refresh also fires on `TurnStart`, so the driver's prompt→turn
  sequence covers both triggers.)
- [ ] **B6 Config overrides.** (Optional, patch `model_display`,
  `cwd_display`, `branch` in the example.) A set override replaces the
  computed value verbatim, including a non-home-relative cwd.

## C. Input editing

- [x] **C1 Typing.** Printable chars, spaces, and unicode (`é`, `中文`,
  `😀`) insert correctly; cursor movement and deletion never split a
  character (backspace over `😀` removes the whole emoji).
- [x] **C2 Movement.** `←`/`→` move by character; `Home`/`End` jump to the
  current *logical* line start/end; `Ctrl+A`/`Ctrl+E` do the same.
- [x] **C3 Deletion.** `Backspace` deletes left, `Delete` deletes right,
  both char-boundary-safe. `Ctrl+H` is not bound (does nothing).
- [x] **C4 `Ctrl+K`.** Cursor mid-line: kills to end of the logical line
  (including any soft-wrapped rows). Cursor just before a newline: joins the
  next line. Repeated presses are no-ops at a dead end.
- [x] **C5 `Ctrl+U`.** Kills the whole current logical line. On a middle
  line it swallows the trailing newline (lines join); on the last line it
  swallows the preceding newline.
- [x] **C6 `Ctrl+W`.** Kills the word (whitespace run + word) left of the
  cursor, e.g. `foo bar|` → `foo |`.
- [x] **C7 `Shift+Enter`.** Inserts a newline (on kitty-capable terminals).
  Type `ab`, Shift+Enter, `cd` → input shows two rows. On Terminal.app the
  key degrades to Enter and submits — expected, not a bug.
- [x] **C8 Alt keys.** `Alt+x` etc. insert nothing (ignored, not stripped
  modifier artifacts).
- [x] **C9 Unknown keys.** `Tab`, function keys, `Ctrl+D` do nothing and
  never crash or move the cursor.
- [x] **C10 Bracketed paste.** Copy multiline text (e.g. three paragraphs
  from any app) and paste (⌘V / Ctrl+Shift+V / right-click). It inserts
  literally with newlines, the input grows to 4 rows then scrolls, and it is
  **never submitted** even if the paste ends in a newline. A CRLF paste
  (e.g. from a Windows file) normalizes to LF.
- [x] **C11 Enter on empty input.** Enter with nothing typed: no event, no
  crash, placeholder remains.
- [x] **C12 Enter on whitespace-only input.** Enter with `   `: currently
  submits nothing **and clears the input** (placeholder returns). Flagged as
  an open question — decide whether whitespace-only should preserve the
  text before finalizing step 1.
- [ ] **C13 History recall (multiline).** Submit `first`, then `second`,
  then a three-line message (`some multi` Shift+Enter `line` Shift+Enter
  `text`), then `fourth`. From an empty input: `↑` recalls `fourth`;
  `↑` again recalls the multiline message with the cursor at its end;
  further `↑` walk the cursor up a display row at a time (`text` →
  `line` → `some multi`); `↑` on the **top row** recalls the previous
  message. Full sequence: `fourth` → `text` → `line` → `some multi` →
  `second` → `first`; `↑` at the oldest does nothing. `↓` mirrors it:
  within a recalled multiline message the cursor walks down a row at a
  time; on the bottom row it recalls the next message, and past the
  newest it restores your original draft. `↓` recalls a message with the
  cursor on its **first line** (so `↑`/`↓` round-tripping into a multiline
  entry shows its start, not its end). Typing any char during recall
  resets it (next `↑` starts from the newest prompt).
- [ ] **C14 Growth and scroll.** The input band grows 1 → 4 rows as content
  grows (long lines wrap at the band width, ~78 cols in an 80-col terminal),
  then scrolls internally with the cursor kept visible — typing past the
  bottom keeps the cursor row in view and the first rows scroll off. While
  scrolled, the **top rule** shows rows hidden above and below,
  right-anchored with a gap column on each side: ` ↑ 2 ↓ 4 ──`
  (mid-scroll), ` ↑ 6 ──` (bottom), ` ↓ 6 ──` (top) — the `──` corner
  stays constant. Both disappear once everything fits again. The
  indicators live on the rule, never overlapping the input text.
- [x] **C15 Wrap edge cases.** A long unbroken string (URL) hard-wraps at
  the right edge mid-word; wide CJK/emoji chars wrap as a unit (never split
  across rows).

## D. Submit, commands, and the channel contract

Runs D1–D5 use the test driver (Appendix A) and `tail -f /tmp/mu-ui-events.log`.

- [x] **D1 Enter submits.** Type `hello world`, Enter. Input clears and the
  placeholder returns; the log gains `UserPrompt: "hello world"`.
- [x] **D2 `/quit`.** Exits cleanly (repeat of A2, here for completeness).
- [x] **D3 Unknown slash command.** Type `/frobnicate`, Enter. It is *not*
  interpreted: the log shows `UserPrompt: "/frobnicate"` and the app keeps
  running.
- [ ] **D4 Esc aborts the turn.** With the driver, submit any prompt: the
  log gains `TurnStart` and the driver opens a 5 s window. Press `Esc`
  inside the window → log gains `Interrupt` and then, promptly, `TurnEnd
  (interrupted)` — the interrupt aborts the simulated turn instead of
  waiting out the timer (the real harness emits `TurnEnd` when a turn is
  cancelled). Press `Esc` again → nothing: the turn is already over. With
  no `Esc`, the window closes on its own — the log gains `TurnEnd` after
  ~5 s, and `Esc` afterwards produces nothing. (`TurnStart` has no
  on-screen effect in step 1 — it only arms `Esc` and refreshes the
  footer — so the log lines are the proof it arrived.)
- [x] **D5 Esc leaves input text alone.** During the window, type text, then
  `Esc`: input text is untouched (only the event fires).
- [x] **D6 Send after exit.** After `/quit`, the driver's final send fails:
  the log ends with `send after exit: Err(...)` (`SendError`), proving the
  channel is closed when the UI exits.

## E. Layout and rendering

- [x] **E1 Edge-to-edge.** The input band has horizontal rules only — no
  `│` side borders, no padding. Option-drag (macOS) / modifier-click text
  selection across the input row captures clean text with no glyphs
  included.
- [x] **E2 Resize.** Resize the window (larger and smaller, and narrower
  than ~10 cols). The rules and footer reflow to the new width, the footer
  stays pinned to the bottom, input re-wraps, the cursor stays on the
  correct row/col, and nothing crashes.
- [x] **E3 Mouse.** Wheel-scroll anywhere does nothing visible (no crash, no
  terminal scrollback interaction). Clicks do not move the input cursor
  (no click-to-place in v1).
- [x] **E4 History pane.** Stays blank across all of the above — history
  items arrive in step 2.

## F. Quick regression pass (30 seconds)

- [x] Launch → type a unicode line → Shift+Enter → second line → `Ctrl+W`,
  `Ctrl+K`, `Ctrl+U` don't corrupt the buffer → submit → `↑` recalls it →
  Enter → `/quit` exits cleanly. (Driver optional; the turn window only
  affects Esc.)

---

## Appendix A: harness-behavior test driver

Replaces the demo's idle driver so prompts open a simulated 5 s turn and
everything is logged to `/tmp/mu-ui-events.log` (watch it with
`tail -f /tmp/mu-ui-events.log` in a second terminal).

```rust
use mu_ui::{AgentEvent, App, AppConfig, UiEvent, Usage};
use std::{
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn log(line: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true).append(true).open("/tmp/mu-ui-events.log")
    {
        let _ = writeln!(file, "{line}");
    }
}

fn main() -> std::io::Result<()> {
    let (app, handle, ui_events) = App::new(AppConfig {
        model_display: "(opencode-go) deepseek-v4-flash".to_string(),
        ..AppConfig::default()
    });

    // Per-turn guard so exactly one TurnEnd is emitted: the interrupt path
    // wins the CAS when the user cancels, the timer thread wins when the
    // turn runs to completion.
    let mut turn_ended = Arc::new(AtomicBool::new(false));

    std::thread::spawn(move || {
        while let Ok(event) = ui_events.recv() {
            match event {
                UiEvent::UserPrompt(text) => {
                    log(&format!("UserPrompt: {text:?}"));
                    log("TurnStart");
                    let _ = handle.send(AgentEvent::TurnStart);
                    let h = handle.clone();
                    let ended = Arc::new(AtomicBool::new(false));
                    turn_ended = ended.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(5));
                        if !ended.swap(true, Ordering::Relaxed) {
                            log("TurnEnd");
                            let _ = h.send(AgentEvent::TurnEnd {
                                usage: Usage { input_tokens: 12_400, output_tokens: 3_100 },
                            });
                        }
                    });
                }
                UiEvent::Interrupt => {
                    log("Interrupt");
                    // Esc cancels the in-flight turn, so the turn ends now
                    // (like the real harness emitting TurnEnd on cancel).
                    if !turn_ended.swap(true, Ordering::Relaxed) {
                        log("TurnEnd (interrupted)");
                        let _ = handle.send(AgentEvent::TurnEnd {
                            usage: Usage::default(),
                        });
                    }
                }
            }
        }
        // UI has exited; this must fail with SendError.
        log(&format!("send after exit: {:?}", handle.send(AgentEvent::TurnStart)));
    });

    app.run()
}
```

Restore the stock demo after the run. (The same driver already lives at
`examples/harness_demo.rs` — run it with `cargo run --example harness_demo`
instead of patching the demo; keep this copy in sync if you change it.)

## Appendix B: panic-restore injection (optional)

Triggers a panic **on the thread running `App::run()`** — panicking on a
spawned thread does not unwind through the guard, so patch `render::draw`
specifically:

```rust
// Temporarily, at the top of pub fn draw(...) in src/render.rs:
if std::env::var("MU_UI_PANIC").is_ok() {
    std::thread::sleep(std::time::Duration::from_millis(800));
    panic!("injected panic");
}
```

```sh
MU_UI_PANIC=1 cargo run --example streaming_demo   # wait ~1 s
```

Expected: the terminal restores (alt screen left, raw mode off, mouse and
keyboard flags popped), the panic message prints, exit code is non-zero, and
the shell is usable afterwards. **Revert the patch** (a debug build will not
compile with it in CI).
