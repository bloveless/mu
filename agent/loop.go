package agent

import (
	"context"
	"errors"
	"fmt"
	"io"
	"strings"

	"github.com/bloveless/mu/api"
	"github.com/bloveless/mu/logging"
	"github.com/bloveless/mu/tools"
)

const MaxIterationsPerUserMessage = 50

type Loop struct {
	Client api.Client
	Model  string
	Tools  tools.Registry
}

func (l Loop) Run(ctx context.Context, messages []api.Message) ([]api.Message, error) {
	for i := range MaxIterationsPerUserMessage {
		res, err := l.streamIteration(ctx, messages, i)
		if err != nil {
			return nil, fmt.Errorf("running streaming iteration: %w", err)
		}
		messages = append(messages, res)
		logging.Log("\n")
		if len(res.ToolCalls) == 0 {
			break // the model is done with this user message
		}
		for _, tc := range res.ToolCalls {
			messages = append(messages, l.Tools.ExecTool(ctx, tc))
			logging.Log("\n")
		}
	}
	return messages, nil
}

// streamIteration executes one turn of the agent loop: it streams a single chat
// completion, renders the response as it arrives, and reassembles any tool
// calls the model requested. The returned assistantMessage (which echoes the
// tool calls, as the API requires) should be appended to the conversation, and
// if toolCalls is non-empty the caller should execute them, append their
// results, and run another iteration.
func (l Loop) streamIteration(
	ctx context.Context,
	messages []api.Message,
	iteration int,
) (api.Message, error) {
	stream, err := l.Client.ChatCompletionStream(ctx, api.ChatCompletionRequest{
		Model:    l.Model,
		Messages: messages,
		Tools:    l.Tools.GetDefinitions(),
		Stream:   true,
	})
	if err != nil {
		return api.Message{}, fmt.Errorf("executing chat completion: %w", err)
	}
	defer stream.Close() //nolint:errcheck // nothing useful to do with a close error here
	var agentResponse strings.Builder
	wasThinking := true
	var finishReason string
	for {
		resp, err := stream.Recv()
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return api.Message{}, fmt.Errorf("reading next message from stream: %w", err)
		}
		if len(resp.Choices) == 0 {
			// some providers send usage-only/keepalive chunks with no choices
			logging.Debug("received chunk with no choices\n")
			continue
		}
		delta := resp.Choices[0].Delta
		if len(delta.ReasoningContent) > 0 {
			logging.ThinkingLog("%s", delta.ReasoningContent)
			wasThinking = true
		}
		if len(delta.Content) > 0 {
			if wasThinking {
				logging.Log("\n\n")
				wasThinking = false
			}
			logging.AssistantLog("%s", delta.Content)
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
