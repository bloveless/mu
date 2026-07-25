// Package render contains user-interface renderers that consume
// events.Event values and decide how to present them.
package render

import (
	"fmt"
	"strings"
	"sync"

	"github.com/bloveless/mu/events"
	"github.com/bloveless/mu/logging"
)

// toolResultMaxLines is how many lines of a tool result are shown before
// truncating with a "… (N more lines)" indicator.
const toolResultMaxLines = 5

// Terminal renders events to stdout with the same colors and spacing the
// agent loop historically produced inline. All presentation state (such as
// whether the last chunk for an agent was thinking or content) lives here,
// per agent, so concurrent subagents can't corrupt each other's layout.
type Terminal struct {
	mu             sync.Mutex
	prompt         string
	last           map[string]events.Kind
	currentAgentID string
}

// NewTerminal creates a terminal renderer that displays the specified prompt when an agent awaits input.
func NewTerminal(prompt string) *Terminal {
	return &Terminal{
		prompt:         prompt,
		last:           make(map[string]events.Kind),
		currentAgentID: "root",
	}
}

// Handle renders a single event from the events channel.
func (t *Terminal) Handle(ev events.Event) {
	t.mu.Lock()
	defer t.mu.Unlock()

	// Detect agent transitions for sub-agent header/footer.
	agentID := ev.AgentID
	if agentID == "" {
		agentID = "root"
	}
	if agentID != t.currentAgentID {
		// Footer for previous sub-agent.
		if t.currentAgentID != "root" {
			logging.Log("\n── END SUBAGENT (%s) ──\n", t.currentAgentID)
		}
		// Header for new sub-agent.
		if agentID != "root" {
			logging.Log("\n── SUBAGENT (%s) ──\n", agentID)
		}
		t.currentAgentID = agentID
	}

	// Choose logging functions — dimmed for sub-agents.
	type logFn func(string, ...any)
	l, tl, al, tol, trl, ul, el, wl := logging.Log, logging.ThinkingLog, logging.AssistantLog,
		logging.ToolLog, logging.ToolResultLog, logging.UsageLog,
		logging.AssistantError, logging.WarningLog
	if agentID != "root" {
		dim := func(fn logFn) logFn {
			return func(msg string, args ...any) {
				fmt.Print("\033[2m")
				fn(msg, args...)
				fmt.Print("\033[0m")
			}
		}
		l, tl, al, tol, trl, ul, el, wl = dim(l), dim(tl), dim(al), dim(tol), dim(trl), dim(ul), dim(el), dim(wl)
	}

	last, seen := t.last[agentID]
	switch ev.Kind {
	case events.KindThinkingDelta:
		tl("%s", ev.Text)
	case events.KindContentDelta:
		// Separate the start of a content block from any preceding
		// thinking (or from the previous turn) with a blank line.
		if !seen || last != events.KindContentDelta {
			l("\n\n")
		}
		al("%s", ev.Text)
	case events.KindToolProgress:
		tol("%s\n", ev.Text)
	case events.KindToolResult:
		trl("%s\n", truncateLines(ev.Text, toolResultMaxLines))
	case events.KindMessageEnd:
		l("\n")
	case events.KindUserMessage:
		// The terminal already echoes what the user typed; separate it
		// from the response with a blank line.
		l("\n\n")
	case events.KindAwaitingInput:
		// After a completed turn, add a blank line before the next prompt.
		if seen && last == events.KindMessageEnd {
			l("\n")
		}
		l("%s", t.prompt)
	case events.KindError:
		el("%s\n", ev.Text)
	case events.KindWarning:
		wl("⚠ %s\n", ev.Text)
	case events.KindUsage:
		ul("\n\n%s\n", ev.Text)
	}
	t.last[agentID] = ev.Kind
}

// truncateLines returns s unchanged when it has maxLines or fewer lines;
// otherwise it returns a "… (N more lines)" indicator followed by the last
// truncateLines removes trailing newlines and limits output to the final maxLines lines.
// It returns "(no output)" for empty input and prefixes truncated output with the number
// of hidden lines.
func truncateLines(s string, maxLines int) string {
	s = strings.TrimRight(s, "\n")
	if s == "" {
		return "(no output)"
	}
	lines := strings.Split(s, "\n")
	if len(lines) <= maxLines {
		return s
	}
	hidden := len(lines) - maxLines
	return fmt.Sprintf("… (%d more lines)\n%s", hidden, strings.Join(lines[hidden:], "\n"))
}
