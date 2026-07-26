package tui

import (
	"strings"
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"

	"github.com/bloveless/mu/events"
)

func newTestModel() (Model, chan string, *RunHandle) {
	inputCh := make(chan string, 1)
	eventCh := make(chan events.Event, 1)
	run := &RunHandle{}
	m := NewModel(inputCh, eventCh, run, "test> ")
	// Give the viewport enough height so history can be read.
	m.height = 40
	m.syncViewportSize()
	return m, inputCh, run
}

func newTestModelWithCh() (Model, chan string, chan events.Event, *RunHandle) {
	inputCh := make(chan string, 1)
	eventCh := make(chan events.Event, 1)
	run := &RunHandle{}
	m := NewModel(inputCh, eventCh, run, "test> ")
	m.height = 40
	m.syncViewportSize()
	return m, inputCh, eventCh, run
}

// subagentBgCode is the ANSI truecolor background sequence for #23200f.
const subagentBgCode = "48;2;35;32;15"

func mustContain(t *testing.T, haystack, needle, label string) {
	t.Helper()
	if !strings.Contains(haystack, needle) {
		t.Fatalf("%s: expected %q to contain %q", label, haystack, needle)
	}
}

func mustNotContain(t *testing.T, haystack, needle, label string) {
	t.Helper()
	if strings.Contains(haystack, needle) {
		t.Fatalf("%s: expected %q NOT to contain %q", label, haystack, needle)
	}
}

func TestDeltaStreaming(t *testing.T) {
	m, _, _ := newTestModel()

	// Partial line stays in the tail; nothing in history yet.
	m.handleEvent(events.Event{AgentID: "root", Kind: events.KindContentDelta, Text: "Hello"})
	if m.tail != "Hello" {
		t.Fatalf("expected tail %q, got %q", "Hello", m.tail)
	}
	if m.history.Len() != 0 {
		t.Fatal("expected empty history for partial line")
	}

	// Completing a line moves it to history; remainder stays in tail.
	m.handleEvent(events.Event{AgentID: "root", Kind: events.KindContentDelta, Text: ", world!\nNext"})
	if m.tail != "Next" {
		t.Fatalf("expected tail %q, got %q", "Next", m.tail)
	}
	mustContain(t, m.history.String(), "Hello, world!", "history missing completed line")

	// MessageEnd flushes the tail to history.
	m.handleEvent(events.Event{AgentID: "root", Kind: events.KindMessageEnd})
	if m.tail != "" {
		t.Fatalf("expected empty tail after message end, got %q", m.tail)
	}
	mustContain(t, m.history.String(), "Next", "history missing flushed tail")
}

func TestThinkingThenContentSeparation(t *testing.T) {
	m, _, _ := newTestModel()
	m.handleEvent(events.Event{AgentID: "root", Kind: events.KindThinkingDelta, Text: "thinking"})
	before := m.history.Len()
	m.handleEvent(events.Event{AgentID: "root", Kind: events.KindContentDelta, Text: "answer"})
	// Kind switch flushes thinking to history, then one blank separator.
	history := m.history.String()
	mustContain(t, history, "thinking", "history missing thinking")
	if !strings.HasSuffix(history, "\n") {
		t.Fatalf("expected trailing newline (blank separator) after thinking flush, got %q", history)
	}
	_ = before
}

func TestSubagentBlock(t *testing.T) {
	m, _, _ := newTestModel()

	// Root content.
	m.handleEvent(events.Event{AgentID: "root", Kind: events.KindContentDelta, Text: "root line\n"})
	mustContain(t, m.history.String(), "root line", "history missing root line")
	mustNotContain(t, m.history.String(), subagentBgCode, "root history should not carry subagent bg")

	// Enter subagent: thinking streams live; nothing in history yet from
	// this event (partial-line thigh).
	m.handleEvent(events.Event{AgentID: "subagent", Kind: events.KindThinkingDelta, Text: "thinking"})
	if !m.subagentActive {
		t.Fatal("expected subagentActive after entering subagent")
	}

	// Subagent content with newline lands in history with bg.
	m.handleEvent(events.Event{AgentID: "subagent", Kind: events.KindContentDelta, Text: "child response\n"})
	history := m.history.String()
	mustContain(t, history, subagentBgCode, "subagent history line missing background")
	mustContain(t, history, "thinking", "subagent history missing thinking")
	mustContain(t, history, "child response", "subagent history missing content")

	// Transition back to root: emits bg-close reset line, then root tool
	// result without bg.
	m.handleEvent(events.Event{AgentID: "root", Kind: events.KindToolResult, Text: "result"})
	history = m.history.String()
	if m.subagentActive {
		t.Fatal("expected subagentActive cleared")
	}
	mustContain(t, history, "result", "history missing root tool result")

	// The root result line itself should not carry bg.  It appears after
	// the reset line, so check that the final line is bg-free.
	lines := strings.Split(history, "\n")
	lastLine := lines[len(lines)-1]
	mustNotContain(t, lastLine, subagentBgCode, "last (root) line should not carry subagent bg")
}

func TestToolResultTruncation(t *testing.T) {
	m, _, _ := newTestModel()
	m.handleEvent(events.Event{AgentID: "root", Kind: events.KindToolResult, Text: "1\n2\n3\n4\n5\n6\n7"})
	mustContain(t, m.history.String(), "… (2 more lines)", "history missing truncation indicator")
}

func TestUsageGoesToFooter(t *testing.T) {
	m, _, _ := newTestModel()
	before := m.history.Len()
	m.handleEvent(events.Event{AgentID: "root", Kind: events.KindUsage, Text: "↑1 ↓2"})
	if m.history.Len() != before {
		t.Fatal("usage should not add to history")
	}
	if got := m.footerView(); !strings.Contains(got, "↑1 ↓2") || strings.Contains(got, "[root]") {
		t.Fatalf("expected plain root usage in footer, got %q", got)
	}
	m.handleEvent(events.Event{AgentID: "subagent", Kind: events.KindUsage, Text: "↑3 ↓4"})
	if got := m.footerView(); !strings.Contains(got, "[subagent] ↑3 ↓4") {
		t.Fatalf("expected tagged subagent usage in footer, got %q", got)
	}
}

func TestAwaitingInputBlankLine(t *testing.T) {
	m, _, _ := newTestModel()
	m.handleEvent(events.Event{AgentID: "root", Kind: events.KindContentDelta, Text: "hi"})
	m.handleEvent(events.Event{AgentID: "root", Kind: events.KindMessageEnd})
	m.handleEvent(events.Event{AgentID: "root", Kind: events.KindAwaitingInput})
	// The flushed tail plus the blank separator from AwaitingInput after
	// MessageEnd should leave a trailing newline = one blank line in history.
	history := m.history.String()
	mustContain(t, history, "hi", "history missing flushed content")
	if !strings.HasSuffix(history, "\n") {
		t.Fatalf("expected trailing newline (blank separator) after awaiting input, got %q", history)
	}
}

func TestCtrlCCancelsRunWhenRunning(t *testing.T) {
	m, _, run := newTestModel()
	cancelled := false
	run.Set(func() { cancelled = true })
	_, cmd := m.Update(tea.KeyPressMsg{Code: 'c', Mod: tea.ModCtrl})
	if !cancelled {
		t.Fatal("expected run cancelled")
	}
	if cmd != nil {
		t.Fatal("expected no cmd when cancelling a run (program stays alive)")
	}
}

func TestCtrlCQuitsWhenIdle(t *testing.T) {
	m, _, _ := newTestModel()
	_, cmd := m.Update(tea.KeyPressMsg{Code: 'c', Mod: tea.ModCtrl})
	if cmd == nil {
		t.Fatal("expected quit cmd when idle")
	}
	if _, ok := cmd().(tea.QuitMsg); !ok {
		t.Fatalf("expected tea.QuitMsg, got %T", cmd())
	}
}

func TestEnterSubmits(t *testing.T) {
	m, inputCh, _ := newTestModel()
	m.textarea.SetValue("hello world")
	updated, cmd := m.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	if got := updated.(Model).textarea.Value(); got != "" {
		t.Fatalf("expected textarea cleared, got %q", got)
	}
	if cmd == nil {
		t.Fatal("expected send cmd")
	}
	// Executing the send cmd delivers the text to the input channel.
	cmd()
	select {
	case sent := <-inputCh:
		if sent != "hello world" {
			t.Fatalf("expected %q sent, got %q", "hello world", sent)
		}
	default:
		t.Fatal("expected input sent to inputCh")
	}
	// Echo line is in history.
	mustContain(t, updated.(Model).historyString(), "> hello world", "history missing echo line")
}

func TestEnterOnEmptyInputDoesNothing(t *testing.T) {
	m, inputCh, _ := newTestModel()
	_, cmd := m.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	if cmd != nil {
		t.Fatal("expected no cmd for empty input")
	}
	select {
	case got := <-inputCh:
		t.Fatalf("expected nothing sent, got %q", got)
	default:
	}
}

func TestSubmitLinesFlushesTailAndEchoes(t *testing.T) {
	m, _, _ := newTestModel()
	m.handleEvent(events.Event{AgentID: "root", Kind: events.KindContentDelta, Text: "streaming"})
	lines := m.submitLines("hi there")
	if len(lines) != 2 {
		t.Fatalf("expected flushed tail + echo line, got %v", lines)
	}
	if !strings.Contains(lines[0], "streaming") {
		t.Fatalf("expected flushed tail first, got %q", lines[0])
	}
	if !strings.Contains(lines[1], "> hi there") {
		t.Fatalf("expected echo line, got %q", lines[1])
	}
	if m.tail != "" {
		t.Fatalf("expected tail cleared, got %q", m.tail)
	}
}

func TestShiftEnterAndAltEnterInsertNewline(t *testing.T) {
	for _, key := range []tea.KeyPressMsg{
		{Code: tea.KeyEnter, Mod: tea.ModShift},
		{Code: tea.KeyEnter, Mod: tea.ModAlt},
	} {
		m, _, _ := newTestModel()
		m.textarea.SetValue("line1")
		updated, _ := m.Update(key)
		if got := updated.(Model).textarea.Value(); !strings.Contains(got, "\n") {
			t.Fatalf("expected %q to insert newline, got %q", key.String(), got)
		}
	}
}

func TestWindowSizeUpdatesWidth(t *testing.T) {
	m, _, _ := newTestModel()
	updated, _ := m.Update(tea.WindowSizeMsg{Width: 123, Height: 50})
	um := updated.(Model)
	if um.width != 123 {
		t.Fatalf("expected width 123, got %d", um.width)
	}
	if um.height != 50 {
		t.Fatalf("expected height 50, got %d", um.height)
	}
	if um.viewport.Width() != 123 {
		t.Fatalf("expected viewport width 123, got %d", um.viewport.Width())
	}
}

func TestSendCmdDeliversInput(t *testing.T) {
	m, inputCh, _ := newTestModel()
	cmd := m.sendCmd("hello")
	cmd()
	select {
	case got := <-inputCh:
		if got != "hello" {
			t.Fatalf("expected %q, got %q", "hello", got)
		}
	case <-time.After(time.Second):
		t.Fatal("timeout waiting for input")
	}
}

func TestEventChClosedQuits(t *testing.T) {
	m, _, _ := newTestModel()
	_, cmd := m.Update(eventChClosedMsg{})
	if cmd == nil {
		t.Fatal("expected quit cmd")
	}
	if _, ok := cmd().(tea.QuitMsg); !ok {
		t.Fatalf("expected tea.QuitMsg, got %T", cmd())
	}
}

func TestViewIncludesTailInputFooter(t *testing.T) {
	m, _, _ := newTestModel()
	// Give the viewport enough height to hold the rendered content.
	m.height = 20
	m.syncViewportSize()
	m.handleEvent(events.Event{AgentID: "root", Kind: events.KindContentDelta, Text: "hello"})
	m.handleEvent(events.Event{AgentID: "root", Kind: events.KindUsage, Text: "↑1 ↓2"})
	content := m.View().Content
	// The tail lives inside the viewport now, not as a separate View line.
	// The viewport renders it as part of its content.
	mustContain(t, content, "test> ", "view missing input prompt")
	mustContain(t, content, "↑1 ↓2", "view missing usage text")
	// Tail text should be in the viewport content.
	mustContain(t, m.history.String()+"\n"+m.tail, "hello", "history+tail missing tail text")
}

func TestEventMsgReturnsCmd(t *testing.T) {
	m, _, eventCh, _ := newTestModelWithCh()
	// eventMsg that produces only a partial line (no history flush)
	// should still return the re-armed awaitEvent cmd.
	_, cmd := m.Update(eventMsg(events.Event{AgentID: "root", Kind: events.KindContentDelta, Text: "hello"}))
	if cmd == nil {
		t.Fatal("expected non-nil cmd (re-armed awaitEvent)")
	}
	// Drain the channel to prove the cmd is a read.
	go func() { eventCh <- events.Event{AgentID: "root", Kind: events.KindUsage, Text: ""} }()
	_, cmd = m.Update(eventMsg(events.Event{AgentID: "root", Kind: events.KindContentDelta, Text: "hello\n"}))
	if cmd == nil {
		t.Fatal("expected non-nil cmd for event producing scrollback")
	}
}

func TestViewTailWrapsToWidth(t *testing.T) {
	m, _, _ := newTestModel()
	m.width = 10
	m.handleEvent(events.Event{AgentID: "root", Kind: events.KindContentDelta, Text: "hello world this is long"})
	// Tail now lives inside viewport content, not as a separate View line.
	// refreshViewport wraps it to width; check that all chunks are present
	// in the viewport content.
	m.refreshViewport()
	content := m.View().Content
	mustContain(t, content, "hello", "viewport missing tail chunk 'hello'")
	mustContain(t, content, "this", "viewport missing tail chunk 'this'")
	mustContain(t, content, "long", "viewport missing tail chunk 'long'")
}
