package agent

import (
	"context"
	"errors"
	"fmt"
	"slices"

	"github.com/tidwall/gjson"

	"github.com/bloveless/mu/api"
	"github.com/bloveless/mu/events"
	"github.com/bloveless/mu/tools"
)

// SubagentTool creates a tool that spawns a synchronous child agent. The
// child runs with its own model, provider, tools (read/edit/bash/fetch
// only), and system prompt. The child emits events on its configured
// Events channel using its own Agent.ID. The child's final assistant
// response is returned as the tool result.
func SubagentTool(
	subAgent *Agent,
) *tools.Tool {
	return &tools.Tool{
		Definition: api.ToolDefinition{
			Type: "function",
			Function: api.FunctionDefinition{
				Name:        "subagent",
				Description: "Spawn a synchronous sub-agent to complete a focused task. The sub-agent has its own context and tools (read, edit, bash, fetch). It runs to completion and returns a comprehensive summary.",
				Parameters: []byte(`{
					"type": "object",
					"required": ["prompt"],
					"properties": {
						"prompt": {
							"type": "string",
							"description": "The task for the sub-agent to complete. Be specific and include what files or areas to investigate."
						}
					}
				}`),
			},
		},
		Exec: func(ctx context.Context, tc api.ToolCall, emit tools.Emitter) api.Message {
			promptResult := gjson.Get(tc.Function.Arguments, "prompt")
			if !promptResult.Exists() {
				return api.NewToolResultMessage(tc.ID, "subagent tool: missing required argument: prompt")
			}
			if promptResult.Type != gjson.String {
				return api.NewToolResultMessage(tc.ID, "subagent tool: prompt must be a string")
			}
			prompt := promptResult.String()
			if prompt == "" {
				return api.NewToolResultMessage(tc.ID, "subagent tool: prompt must not be empty")
			}

			emit(ctx, events.KindToolProgress, fmt.Sprintf("spawning sub-agent (%s:%s)...", subAgent.Provider.ID, subAgent.Model.ID))

			session := subAgent.NewSession(ctx)
			err := session.ExecutePrompt(ctx, prompt)
			if err != nil && !errors.Is(err, ErrMaxIterationsReached) {
				return api.NewToolResultMessage(tc.ID, fmt.Sprintf("subagent failed: %s", err))
			}

			// Find the last assistant message.
			for _, msg := range slices.Backward(session.Messages()) {
				if msg.Role == api.RoleAssistant && msg.Content != "" {
					content := msg.Content
					if errors.Is(err, ErrMaxIterationsReached) {
						content = fmt.Sprintf("[TRUNCATED: sub-agent reached iteration cap (%d) with tool calls still pending]\n\n%s", subAgent.MaxIterations, content)
					}
					return api.NewToolResultMessage(tc.ID, content)
				}
			}
			if errors.Is(err, ErrMaxIterationsReached) {
				return api.NewToolResultMessage(tc.ID, fmt.Sprintf("[TRUNCATED: sub-agent reached iteration cap (%d) with tool calls still pending and produced no output]", subAgent.MaxIterations))
			}
			return api.NewToolResultMessage(tc.ID, "subagent completed but produced no output")
		},
	}
}
