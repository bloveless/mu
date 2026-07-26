package agent

import (
	"context"
	"errors"
	"fmt"
	"io"
	"strings"

	"github.com/bloveless/mu/api"
	"github.com/bloveless/mu/events"
	"github.com/bloveless/mu/logging"
	"github.com/bloveless/mu/tools"
)

// Agent represents a conversational agent that can execute prompts and manage
// tool calls.
type Agent struct {
	ID            string
	Client        api.Client
	MaxIterations int
	Model         api.ProviderModel
	Provider      api.Provider
	ToolsRegistry tools.Registry
	// SystemPrompt provides the identity of this agent and tools instructions project guidance should be in AGENTS.md
	SystemPrompt string
	Events       chan<- events.Event
}

// Emit sends an event to the agent's event channel, typically for CLI rendering.
func (a *Agent) Emit(ctx context.Context, kind events.Kind, text string) {
	select {
	case <-ctx.Done():
		return
	case a.Events <- events.Event{AgentID: a.ID, Kind: kind, Text: text}:
	}
}

// NewSession creates a new conversation session for the agent. Agents can create new sessions at any time and the
// conversational history/context will be cleared between sessions.
func (a *Agent) NewSession(ctx context.Context) *Session {
	var messages []api.Message

	if a.SystemPrompt != "" {
		messages = append(messages, api.Message{Role: api.RoleSystem, Content: a.SystemPrompt})
	}
	facts := getFacts()
	messages = append(messages, api.Message{Role: api.RoleSystem, Content: facts.String()})
	agentInst, instSize := getAgentInstructions(ctx)
	if instSize > 25_000 {
		a.Emit(ctx, events.KindWarning, fmt.Sprintf(
			"AGENTS.md is %d KB — large instruction files increase token usage and may crowd out conversation context.",
			instSize/1024,
		))
	}
	if agentInst != "" {
		messages = append(messages, api.Message{
			Role: api.RoleSystem, Content: fmt.Sprintf(
				`These are the instructions on how you should interact with this project.
	Follow them closely and only deviate from them if the user specifically asks it.
	If you are unsure and run into any contradictions then ask the user what to do:

	# AGENTS.md
	%s`,
				agentInst,
			),
		})
	}

	return &Session{
		agentID:       a.ID,
		client:        a.Client,
		model:         a.Model,
		toolsRegistry: a.ToolsRegistry,
		emit:          a.Emit,
		maxIterations: a.MaxIterations,
		messages:      messages,
	}
}

// Session represents a single conversation session with the agent with cost tracking built in.
type Session struct {
	agentID        string
	model          api.ProviderModel
	client         api.Client
	toolsRegistry  tools.Registry
	emit           func(context.Context, events.Kind, string)
	maxIterations  int
	messages       []api.Message
	cumulativeCost float64
}

// ErrMaxIterationsReached is returned by ExecutePrompt when the turn is
// truncated because the iteration cap was reached with tool calls still
// pending. The conversation is intact and another message may be sent to
// continue.
var ErrMaxIterationsReached = errors.New("max iterations reached")

// ExecutePrompt runs a single prompt through the agent, streaming the response
// and executing tool calls as needed. It will continue iterating until the
// model is done (no more tool calls or has returned content) or the iteration
// cap is reached.
func (s *Session) ExecutePrompt(ctx context.Context, prompt string) error {
	s.messages = append(s.messages, api.NewUserMessage(prompt))
	for i := range s.maxIterations {
		res, err := s.streamIteration(ctx, i)
		if err != nil {
			return fmt.Errorf("running streaming iteration: %w", err)
		}
		s.messages = append(s.messages, res)
		if len(res.ToolCalls) == 0 {
			return nil // the model is done with this user message
		}
		for _, tc := range res.ToolCalls {
			result := s.toolsRegistry.ExecTool(ctx, tc, s.emit)
			s.messages = append(s.messages, result)
			s.emit(ctx, events.KindToolResult, result.Content)
			s.emit(ctx, events.KindMessageEnd, "")
		}
	}
	msg := fmt.Sprintf("reached iteration cap (%d) with tool calls still pending; turn truncated — the conversation is intact, send another message to continue", s.maxIterations)
	s.emit(ctx, events.KindError, msg)
	return ErrMaxIterationsReached
}

// Messages returns the current conversation state as a slice of messages.
func (s *Session) Messages() []api.Message {
	return s.messages
}

// streamIteration executes one turn of the agent loop: it streams a single chat
// completion, emits display events as the response arrives, and reassembles
// any tool calls the model requested. The returned assistantMessage (which echoes the
// tool calls, as the API requires) should be appended to the conversation, and
// if toolCalls is non-empty the caller should execute them, append their
// results, and run another iteration.
func (s *Session) streamIteration(
	ctx context.Context,
	iteration int,
) (api.Message, error) {
	stream, err := s.client.ChatCompletionStream(ctx, api.ChatCompletionRequest{
		Model:         s.model.ID,
		Messages:      s.messages,
		Tools:         s.toolsRegistry.GetDefinitions(),
		Stream:        true,
		StreamOptions: api.StreamOptions{IncludeUsage: true},
	})
	if err != nil {
		return api.Message{}, fmt.Errorf("executing chat completion: %w", err)
	}
	defer stream.Close() //nolint:errcheck // nothing useful to do with a close error here
	var agentResponse strings.Builder
	var finishReason string
	costAtStart := s.cumulativeCost
	var lastUsage *api.Usage
	for {
		resp, err := stream.Recv()
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return api.Message{}, fmt.Errorf("reading next message from stream: %w", err)
		}
		if resp.Usage != nil {
			lastUsage = resp.Usage
			s.emit(ctx, events.KindUsage, FormatUsageLine(resp.Usage, &s.model, costAtStart+computeCost(resp.Usage, &s.model)))
		}
		if len(resp.Choices) == 0 {
			// some providers send usage-only/keepalive chunks with no choices
			logging.Debug("received chunk with no choices\n")
			continue
		}
		delta := resp.Choices[0].Delta
		if len(delta.ReasoningContent) > 0 {
			s.emit(ctx, events.KindThinkingDelta, delta.ReasoningContent)
		}
		if len(delta.Content) > 0 {
			s.emit(ctx, events.KindContentDelta, delta.Content)
			agentResponse.WriteString(delta.Content)
		}
		if resp.Choices[0].FinishReason != "" {
			finishReason = resp.Choices[0].FinishReason
		}
	}
	logging.Debug("iteration %d; finish reason: %s\n", iteration, finishReason)
	if finishReason == "length" {
		logging.Error("iteration %d: response truncated by token limit (finish reason: length)\n", iteration)
	}
	// Persist this response's cost into the session cumulative total.
	if lastUsage != nil {
		s.cumulativeCost += computeCost(lastUsage, &s.model)
	}
	calls := stream.ToolCalls()
	if agentResponse.Len() == 0 && len(calls) == 0 {
		// Some reasoning models return their entire response as
		// ReasoningContent with zero Content. When the stream
		// ended normally (finishReason is set), accept it as an
		// empty assistant message so the turn ends gracefully
		// instead of crashing the program.
		if finishReason != "" {
			return api.Message{Role: api.RoleAssistant}, nil
		}
		return api.Message{}, fmt.Errorf("iteration %d: provider returned an empty response", iteration)
	}
	return api.Message{
		Role:      api.RoleAssistant,
		Content:   agentResponse.String(),
		ToolCalls: calls,
	}, nil
}
