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
	mu     sync.Mutex
	prompt string
	last   map[string]events.Kind
}

// NewTerminal creates a terminal renderer that shows prompt whenever the
// NewTerminal creates a terminal renderer that displays the specified prompt when an agent awaits input.
func NewTerminal(prompt string) *Terminal {
	return &Terminal{prompt: prompt, last: make(map[string]events.Kind)}
}

// Handle renders a single event from the events channel.
func (t *Terminal) Handle(ev events.Event) {
	t.mu.Lock()
	defer t.mu.Unlock()
	last, seen := t.last[ev.AgentID]
	switch ev.Kind {
	case events.KindThinkingDelta:
		logging.ThinkingLog("%s", ev.Text)
	case events.KindContentDelta:
		// Separate the start of a content block from any preceding
		// thinking (or from the previous turn) with a blank line.
		if !seen || last != events.KindContentDelta {
			logging.Log("\n\n")
		}
		logging.AssistantLog("%s", ev.Text)
	case events.KindToolProgress:
		logging.ToolLog("%s\n", ev.Text)
	case events.KindToolResult:
		logging.ToolResultLog("%s\n", truncateLines(ev.Text, toolResultMaxLines))
	case events.KindMessageEnd:
		logging.Log("\n")
	case events.KindUserMessage:
		// The terminal already echoes what the user typed; separate it
		// from the response with a blank line.
		logging.Log("\n\n")
	case events.KindAwaitingInput:
		// After a completed turn, add a blank line before the next prompt.
		if seen && last == events.KindMessageEnd {
			logging.Log("\n")
		}
		logging.Log("%s", t.prompt)
	case events.KindError:
		logging.AssistantError("%s\n", ev.Text)
	case events.KindUsage:
		logging.UsageLog("\n\n%s\n", ev.Text)
	}
	t.last[ev.AgentID] = ev.Kind
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
