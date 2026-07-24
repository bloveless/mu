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

// InputKind identifies the type of an Input submitted to the agent session.
type InputKind int

const (
	// InputKindUserMessage is a user message to append to the conversation
	// and run a turn for.
	InputKindUserMessage InputKind = iota
)

// Input is a single submission to a running agent Loop. Input adapters
// (a stdin REPL, Bubble Tea key handling, JSON-RPC) translate their
// medium into Inputs; the agent doesn't know or care which medium
// produced them.
type Input struct {
	Kind InputKind
	Text string
}

type Loop struct {
	AgentID       string
	Client        api.Client
	MaxIterations int
	Model         api.ProviderModel
	Provider      api.Provider
	ToolsRegistry tools.Registry
	// SystemPrompt provides the identity of this agent and tools instructions project guidance should be in AGENTS.md
	SystemPrompt string
	// AgentInstructions provider project level guidance for how the agent should interact with this project.
	AgentInstructions string
	Events            chan events.Event

	// CumulativeCost tracks the total USD cost of all API calls in this
	// session, accumulated across turns and tool-call iterations.
	CumulativeCost float64
}

// // emit pushes a display event onto the sink's channel.
// func (l *Loop) emit(ctx context.Context, kind events.Kind, text string) {
// 	l.Sink.Send(ctx, kind, text)
// }

func (l *Loop) emit(ctx context.Context, kind events.Kind, text string) {
	select {
	case <-ctx.Done():
		return
	case l.Events <- events.Event{AgentID: l.AgentID, Kind: kind, Text: text}:
	}
}

// Run starts the agent session: it seeds the conversation with the system
// prompt, then consumes Inputs until the channel is closed or ctx is
// cancelled, running a full turn (model + tool iterations) per user
// message. The session owns the conversation state; input and rendering
// adapters live outside the agent.
func (l *Loop) Run(ctx context.Context, in <-chan Input) error {
	var messages []api.Message
	if l.SystemPrompt != "" {
		messages = append(messages, api.Message{Role: api.RoleSystem, Content: l.SystemPrompt})
	}
	if l.AgentInstructions != "" {
		messages = append(messages, api.Message{Role: api.RoleSystem, Content: fmt.Sprintf(
			`These are the instructions on how you should interact with this project.
Follow them closely and only deviate from them if the user specifically asks it.
If you are unsure and run into any contradictions then ask the user what to do:

# AGENTS.md
%s`,
			l.AgentInstructions,
		)})
	}
	for {
		l.emit(ctx, events.KindAwaitingInput, "")
		select {
		case <-ctx.Done():
			return nil
		case input, ok := <-in:
			if !ok {
				return nil // the input source closed; the session is over
			}
			switch input.Kind {
			case InputKindUserMessage:
				if strings.TrimSpace(input.Text) == "" {
					continue // re-emit AwaitingInput without running a turn
				}
				l.emit(ctx, events.KindUserMessage, input.Text)
				var err error
				messages, err = l.runTurn(ctx, messages, input.Text)
				if err != nil {
					return fmt.Errorf("running turn: %w", err)
				}
			default:
				logging.Debug("ignoring unknown input kind: %d\n", input.Kind)
			}
		}
	}
}

// runTurn executes the iteration loop for a single user message: it
// streams completions, executes any requested tool calls, and repeats
// until the model stops calling tools or the iteration cap is hit. It
// returns the updated conversation.
func (l *Loop) runTurn(ctx context.Context, messages []api.Message, userMessage string) ([]api.Message, error) {
	messages = append(messages, api.NewUserMessage(userMessage))
	for i := range l.MaxIterations {
		res, err := l.streamIteration(ctx, messages, i)
		if err != nil {
			return nil, fmt.Errorf("running streaming iteration: %w", err)
		}
		messages = append(messages, res)
		l.emit(ctx, events.KindMessageEnd, "")
		if len(res.ToolCalls) == 0 {
			return messages, nil // the model is done with this user message
		}
		for _, tc := range res.ToolCalls {
			result := l.ToolsRegistry.ExecTool(ctx, tc, l.emit)
			messages = append(messages, result)
			l.emit(ctx, events.KindToolResult, result.Content)
			l.emit(ctx, events.KindMessageEnd, "")
		}
	}
	msg := fmt.Sprintf("reached iteration cap (%d) with tool calls still pending; turn truncated — the conversation is intact, send another message to continue", l.MaxIterations)
	l.emit(ctx, events.KindError, msg)
	l.emit(ctx, events.KindMessageEnd, "")
	return messages, nil
}

// streamIteration executes one turn of the agent loop: it streams a single chat
// completion, emits display events as the response arrives, and reassembles
// any tool calls the model requested. The returned assistantMessage (which echoes the
// tool calls, as the API requires) should be appended to the conversation, and
// if toolCalls is non-empty the caller should execute them, append their
// results, and run another iteration.
func (l *Loop) streamIteration(
	ctx context.Context,
	messages []api.Message,
	iteration int,
) (api.Message, error) {
	stream, err := l.Client.ChatCompletionStream(ctx, api.ChatCompletionRequest{
		Model:         l.Model.ID,
		Messages:      messages,
		Tools:         l.ToolsRegistry.GetDefinitions(),
		Stream:        true,
		StreamOptions: api.StreamOptions{IncludeUsage: true},
	})
	if err != nil {
		return api.Message{}, fmt.Errorf("executing chat completion: %w", err)
	}
	defer stream.Close() //nolint:errcheck // nothing useful to do with a close error here
	var agentResponse strings.Builder
	var finishReason string
	costAtStart := l.CumulativeCost
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
			l.emit(ctx, events.KindUsage, FormatUsageLine(resp.Usage, &l.Model, costAtStart+computeCost(resp.Usage, &l.Model)))
		}
		if len(resp.Choices) == 0 {
			// some providers send usage-only/keepalive chunks with no choices
			logging.Debug("received chunk with no choices\n")
			continue
		}
		delta := resp.Choices[0].Delta
		if len(delta.ReasoningContent) > 0 {
			l.emit(ctx, events.KindThinkingDelta, delta.ReasoningContent)
		}
		if len(delta.Content) > 0 {
			l.emit(ctx, events.KindContentDelta, delta.Content)
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
		l.CumulativeCost += computeCost(lastUsage, &l.Model)
	}
	calls := stream.ToolCalls()
	if agentResponse.Len() == 0 && len(calls) == 0 {
		return api.Message{}, fmt.Errorf("iteration %d: provider returned an empty response", iteration)
	}
	return api.Message{
		Role:      api.RoleAssistant,
		Content:   agentResponse.String(),
		ToolCalls: calls,
	}, nil
}
