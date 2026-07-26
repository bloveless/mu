// Package tui implements the Bubble Tea frontend for mu. The model is both
// ends of the agent pipeline: it produces user inputs (replacing the CLI's
// stdin adapter) and consumes display events (replacing render.Terminal).
// The alt-screen viewport gives us control over rendering so sub-agent
// output can carry a continuous background without the per-line banding
// that happened with tea.Println's line-atomic insertAbove writes.
package tui

import (
	"strings"

	"charm.land/bubbles/v2/textarea"
	"charm.land/bubbles/v2/viewport"
	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"

	"github.com/bloveless/mu/events"
	"github.com/bloveless/mu/render"
)

// toolResultMaxLines mirrors the CLI's truncation of tool results.
const toolResultMaxLines = 5

// subagentBackground is the sub-agent block background: a dark black with a
// light yellow hue. It only needs to be subtly distinguishable from the
// terminal's default background — tweak freely.
var subagentBackground = lipgloss.Color("#23200f")

// Foreground styles mirror the CLI colors in logging/logger.go.
var (
	thinkingStyle   = lipgloss.NewStyle().Foreground(lipgloss.Color("3"))   // yellow
	contentStyle    = lipgloss.NewStyle()                                   // terminal default
	toolStyle       = lipgloss.NewStyle().Foreground(lipgloss.Color("6"))   // cyan
	toolResultStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("90"))  // grey
	usageStyle      = lipgloss.NewStyle().Foreground(lipgloss.Color("5"))   // purple
	warningStyle    = lipgloss.NewStyle().Foreground(lipgloss.Color("208")) // orange
	errorStyle      = lipgloss.NewStyle().Foreground(lipgloss.Color("1"))   // red
	promptStyle     = lipgloss.NewStyle().Foreground(lipgloss.Color("4"))   // blue
)

// eventMsg wraps an events.Event as a tea.Msg.
type eventMsg events.Event

// eventChClosedMsg signals the agent's event channel closed; the program
// shuts down.
type eventChClosedMsg struct{}

// awaitEvent reads the next agent event and returns it as a tea.Msg. It is
// re-issued after every handled event, keeping exactly one read in flight.
func awaitEvent(ch <-chan events.Event) tea.Cmd {
	return func() tea.Msg {
		ev, ok := <-ch
		if !ok {
			return eventChClosedMsg{}
		}
		return eventMsg(ev)
	}
}

// Model is the bubbletea model for the mu TUI.
type Model struct {
	inputCh  chan<- string
	eventCh  <-chan events.Event
	run      *RunHandle
	textarea textarea.Model
	viewport viewport.Model

	width  int
	height int

	// Rendering state ported from render.Terminal: per-agent last-kind
	// tracking, plus the tail — the in-progress streamed line for the
	// agent currently streaming.  The tail is included as the last
	// line(s) of the viewport content so it stays in place when it
	// graduates to completed history.
	last           map[string]events.Kind
	currentAgentID string
	tail           string
	tailKind       events.Kind

	// subagentActive is set while the current agent is not root.  While
	// active, scrollback lines carry the subagent background, and the
	// trailing  (reset-all) is replaced with  (reset-
	// foreground-only) so the background persists across newlines in the
	// viewport content stream — this gives a continuous background block
	// without per-line banding.
	subagentActive bool

	// history accumulates completed scrollback lines.  A pointer avoids
	// strings.Builder's copy-on-write guard tripping when bubbletea
	// copies the Model value between Update frames.
	history *strings.Builder

	usage      string // latest usage line, shown in the footer
	usageAgent string
}

// NewModel creates the TUI model. inputCh carries submitted prompts to the
// agent loop; eventCh delivers agent display events; run lets ctrl+c cancel
// the in-flight turn; prompt is the input prompt string.
func NewModel(inputCh chan<- string, eventCh <-chan events.Event, run *RunHandle, prompt string) Model {
	ta := textarea.New()
	s := ta.Styles()
	s.Focused.Prompt = lipgloss.NewStyle().Foreground(lipgloss.Color("4"))
	s.Blurred.Prompt = s.Focused.Prompt
	ta.SetStyles(s)
	ta.Prompt = prompt
	ta.ShowLineNumbers = false
	ta.SetHeight(1)
	// Plain enter submits (handled in Update); newlines come from
	// shift+enter / alt+enter.
	ta.KeyMap.InsertNewline.SetKeys("shift+enter", "alt+enter")
	_ = ta.Focus() // the focus cmd only starts cursor blink; not needed

	vp := viewport.New()
	vp.SoftWrap = true

	return Model{
		inputCh:        inputCh,
		eventCh:        eventCh,
		run:            run,
		textarea:       ta,
		viewport:       vp,
		width:          80,
		height:         24,
		last:           make(map[string]events.Kind),
		currentAgentID: "root",
		history:        &strings.Builder{},
	}
}

// Init starts the event pump.
func (m Model) Init() tea.Cmd {
	return awaitEvent(m.eventCh)
}

// appendHistory adds one styled line to completed scrollback and refreshes
// the viewport.
func (m *Model) appendHistory(line string) {
	if m.history.Len() > 0 {
		m.history.WriteByte('\n')
	}
	m.history.WriteString(line)
}

// refreshViewport rebuilds the viewport content from completed history plus
// the live tail (if any), then resizes and re-scrolls the viewport.
// Called after every event so the tail appears inline with history.
func (m *Model) refreshViewport() {
	content := m.history.String()
	if m.tail != "" {
		if content != "" {
			content += "\n"
		}
		styled := m.styleFor(m.currentAgentID, m.tailKind).Render(m.tail)
		wrapped := lipgloss.NewStyle().Width(m.width).Render(styled)
		content += wrapped
	}
	atBottom := m.viewport.AtBottom()
	m.viewport.SetContent(content)
	m.syncViewportSize()
	if atBottom {
		m.viewport.GotoBottom()
	}
}

// renderHistoryLine returns a styled scrollback line.  For root-agent lines
// this is the CLI's plain foreground style.  For sub-agent lines it adds
// the block background + full-width padding, and strips the final ANSI
// reset so the background persists through the newlines in the viewport
// content stream — the next sub-agent line opens the background again
// harmlessly, and a bare \x1b[m close-line is emitted when the sub-agent
// finishes.
func (m *Model) renderHistoryLine(agentID string, kind events.Kind, text string) string {
	s := kindStyle(kind)
	if agentID == "root" {
		return s.Render(text)
	}
	s = s.Background(subagentBackground).Width(m.width)
	rendered := s.Render(text)
	if strings.HasSuffix(rendered, "\x1b[m") {
		rendered = rendered[:len(rendered)-3] + "\x1b[39m"
	}
	return rendered
}

// handleEvent processes one agent display event, updating the tail, the
// scrollback history, and the footer.  Rendering logic is ported from
// render.Terminal with the same foreground colors; usage events update the
// footer instead of printing.
func (m *Model) handleEvent(ev events.Event) {
	agentID := ev.AgentID
	if agentID == "" {
		agentID = "root"
	}

	// Agent transition: flush any streaming output.  When leaving a
	// sub-agent, close the background with a bare reset line so
	// subsequent root output renders on the normal terminal background.
	if agentID != m.currentAgentID {
		if m.subagentActive {
			m.flush()
			m.appendHistory("\x1b[m")
			m.subagentActive = false
		} else {
			m.flush()
		}
		if agentID != "root" {
			m.subagentActive = true
		}
		m.currentAgentID = agentID
	}

	switch ev.Kind {
	case events.KindThinkingDelta, events.KindContentDelta:
		if m.tail != "" && m.tailKind != ev.Kind {
			m.flush()
		}
		if m.tail == "" {
			if ev.Kind == events.KindContentDelta {
				// Separate a new content block from previous
				// output with a blank line.
				if last, seen := m.last[agentID]; seen && last != events.KindContentDelta {
					m.appendHistory("")
				}
			}
			m.tailKind = ev.Kind
		}
		m.tail += ev.Text
		m.printCompleteLines()

	case events.KindToolProgress, events.KindToolResult, events.KindError, events.KindWarning:
		m.flush()
		text := ev.Text
		if ev.Kind == events.KindToolResult {
			text = render.TruncateLines(text, toolResultMaxLines)
		}
		if ev.Kind == events.KindWarning {
			text = "⚠ " + text
		}
		m.appendHistory(m.renderHistoryLine(agentID, ev.Kind, text))

	case events.KindMessageEnd:
		m.flush()

	case events.KindUserMessage:
		m.flush()
		m.appendHistory("")

	case events.KindAwaitingInput:
		m.flush()
		if last, seen := m.last[agentID]; seen && last == events.KindMessageEnd {
			m.appendHistory("")
		}

	case events.KindUsage:
		m.usage = ev.Text
		m.usageAgent = agentID
	}

	m.last[agentID] = ev.Kind
	m.refreshViewport()
}

// printCompleteLines moves complete lines (everything up to the last
// newline) from the tail into the scrollback history, leaving the
// in-progress remainder in the tail for the live region.
func (m *Model) printCompleteLines() {
	i := strings.LastIndex(m.tail, "\n")
	if i < 0 {
		return
	}
	complete, rest := m.tail[:i], m.tail[i+1:]
	m.tail = rest
	for _, l := range strings.Split(complete, "\n") {
		m.appendHistory(m.renderHistoryLine(m.currentAgentID, m.tailKind, l))
	}
}

// flush prints the remaining tail as a final scrollback line and clears it.
// Called at block boundaries and agent transitions.
func (m *Model) flush() {
	if m.tail == "" {
		return
	}
	m.appendHistory(m.renderHistoryLine(m.currentAgentID, m.tailKind, m.tail))
	m.tail = ""
}

// styleFor returns the render style for an event kind from the given agent:
// the CLI's foreground colors, and for sub-agents the distinguishing
// background padded to full width.  Used for the live tail inside the
// viewport and for the old live-region tail.
func (m *Model) styleFor(agentID string, kind events.Kind) lipgloss.Style {
	s := kindStyle(kind)
	if agentID != "root" {
		s = s.Background(subagentBackground).Width(m.width)
	}
	return s
}

// kindStyle maps an event kind to the CLI's foreground color for it.
func kindStyle(kind events.Kind) lipgloss.Style {
	switch kind {
	case events.KindThinkingDelta:
		return thinkingStyle
	case events.KindToolProgress:
		return toolStyle
	case events.KindToolResult:
		return toolResultStyle
	case events.KindError:
		return errorStyle
	case events.KindWarning:
		return warningStyle
	default:
		return contentStyle
	}
}

// footerView renders the footer line: the latest usage report, tagged with
// the agent that produced it when it isn't the root agent.
func (m *Model) footerView() string {
	text := m.usage
	if m.usageAgent != "" && m.usageAgent != "root" {
		text = "[" + m.usageAgent + "] " + text
	}
	return usageStyle.Render(text)
}

// View renders the alt-screen UI: scrollable history (viewport), the
// one-line input, and the usage footer.  The live streaming tail is
// included in the viewport content so it stays in place when it
// graduates to completed history.
func (m Model) View() tea.View {
	var b strings.Builder
	b.WriteString(m.viewport.View())
	b.WriteByte('\n')
	b.WriteString(m.textarea.View())
	b.WriteByte('\n')
	b.WriteString(m.footerView())
	v := tea.NewView(b.String())
	v.AltScreen = true
	return v
}

// syncViewportSize sets the viewport dimensions from the current model
// width and height, reserving space for the input and footer.
func (m *Model) syncViewportSize() {
	m.viewport.SetWidth(m.width)
	h := m.height - 2 // textarea + footer
	if h < 1 {
		h = 1
	}
	m.viewport.SetHeight(h)
}

// historyString is a value-receiver helper so callers with non-addressable
// Model values (e.g. type-assertion results) can read the scrollback.
func (m Model) historyString() string { return m.history.String() }

// Update handles tea messages: agent events, window resizes, and keyboard
// input.  Keys we don't intercept are delegated to the textarea (which
// inserts newlines on shift+enter/alt+enter and handles paste) and to the
// viewport (which handles pgup/pgdn/mouse-wheel scrolling).
func (m Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyPressMsg:
		switch msg.String() {
		case "ctrl+c":
			if m.run.Cancel() {
				return m, nil
			}
			return m, tea.Quit
		case "enter":
			return m.submit()
		}
	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		m.textarea.SetWidth(msg.Width)
		m.refreshViewport()
		m.viewport.GotoBottom()
		return m, nil
	case tea.InterruptMsg, eventChClosedMsg:
		return m, tea.Quit
	case eventMsg:
		m.handleEvent(events.Event(msg))
		return m, awaitEvent(m.eventCh)
	}

	// Unhandled messages (unintercepted keys, paste, mouse wheel, …)
	// go to both components: textarea for typing and input navigation,
	// viewport for history scrolling.
	var taCmd, vpCmd tea.Cmd
	m.textarea, taCmd = m.textarea.Update(msg)
	m.viewport, vpCmd = m.viewport.Update(msg)
	return m, tea.Batch(taCmd, vpCmd)
}

// sendCmd returns a command that writes text to the agent input channel.
func (m Model) sendCmd(text string) tea.Cmd {
	ch := m.inputCh
	return func() tea.Msg { ch <- text; return nil }
}

// submit echoes the input into the scrollback history, sends it to the
// agent, and clears the textarea.
func (m *Model) submit() (tea.Model, tea.Cmd) {
	text := strings.TrimSpace(m.textarea.Value())
	if text == "" {
		return *m, nil
	}
	m.textarea.Reset()
	m.flush()
	m.appendHistory(promptStyle.Render("> " + text))
	m.refreshViewport()
	return *m, m.sendCmd(text)
}

// submitLines returns the scrollback lines for a submit: any in-progress
// streamed output first (so history stays in order), then the echoed user
// input.
func (m *Model) submitLines(text string) []string {
	return append(m.flushLines(), promptStyle.Render("> "+text))
}

// flushLines is like flush but returns the line instead of appending to
// history — used by submitLines for testability.
func (m *Model) flushLines() []string {
	if m.tail == "" {
		return nil
	}
	line := m.renderHistoryLine(m.currentAgentID, m.tailKind, m.tail)
	m.tail = ""
	return []string{line}
}
