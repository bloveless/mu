// Package events defines the display-event contract between the agent loop
// (and tools) and any user-interface renderer. Producers push structured
// events onto a channel describing what happened; renderers (terminal,
// Bubble Tea, test recorders) consume the channel and decide how — or
// whether — to present them. This package has no UI dependencies so
// events can later double as tea.Msg values.
package events

// Kind identifies what sort of thing happened. Producers should emit raw
// facts (full text, untruncated results); presentation concerns like
// truncation, color, and separators belong to the renderer.
type Kind int

const (
	// KindThinkingDelta is a chunk of the model's reasoning stream.
	KindThinkingDelta Kind = iota
	// KindContentDelta is a chunk of the model's visible response stream.
	KindContentDelta
	// KindToolProgress is a human-readable progress note from a tool
	// (e.g. "reading file: x").
	KindToolProgress
	// KindToolResult carries the full, untruncated content of a tool result.
	KindToolResult
	// KindMessageEnd marks a message (assistant or tool result) having been
	// appended to the conversation.
	KindMessageEnd
	// KindUserMessage marks a user message having been submitted to the
	// agent. Text carries the message.
	KindUserMessage
	// KindAwaitingInput marks the agent as idle and ready to consume the
	// next input. Renderers use it to know when to show a prompt.
	KindAwaitingInput
	// KindError is a general error message that needs to be displayed to the user
	KindError
	// KindUsage marks a usage message from the model, e.g. token usage information
	KindUsage
)

// Event is a single displayable occurrence. AgentID identifies which agent
// produced it ("" or "root" for the main agent); subagents stamp their own
// ID so renderers can indent, collapse, or hide their output.
type Event struct {
	AgentID string
	Kind    Kind
	Text    string
}
