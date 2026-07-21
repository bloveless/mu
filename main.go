package main

import (
	"bufio"
	"context"
	_ "embed"
	"errors"
	"flag"
	"fmt"
	"io"
	"net/url"
	"os"
	"strings"

	"github.com/bloveless/mu/api"
	"github.com/bloveless/mu/logging"
)

const MaxIterationsPerUserMessage = 50

//go:embed DEFAULT_INSTRUCTIONS.md
var DefaultInstructions string

func main() {
	verbose := flag.Bool("v", false, "enable debug logging")
	flag.Parse()

	if err := run(*verbose); err != nil {
		logging.Error("error running mu: %s\n", err)
		os.Exit(1)
	}
}

func run(verbose bool) error {
	logging.Init(verbose)

	ctx := context.Background()
	providers, err := api.GetProviders(ctx)
	if err != nil {
		return fmt.Errorf("refreshing models: %w", err)
	}
	provider, ok := providers["opencode"]
	if !ok {
		return fmt.Errorf("unable to find provider in providers.json")
	}
	model := "deepseek-v4-flash-free"
	baseURL, err := url.Parse(provider.API)
	if err != nil {
		return fmt.Errorf("parsing base URL: %w", err)
	}
	if len(provider.Env) != 1 {
		return fmt.Errorf(
			"provider didn't have exactly one environment variable in models.dev... not quite sure what to do with that",
		)
	}
	apiKey := os.Getenv(provider.Env[0])
	if apiKey == "" {
		return fmt.Errorf("unable to find required API key [%s] in environment", provider.Env[0])
	}
	c := api.NewClient(baseURL, apiKey)
	tools, err := toolRegistry()
	if err != nil {
		return fmt.Errorf("getting tool registry: %w", err)
	}

	messages := []api.Message{
		{
			Role:    api.RoleSystem,
			Content: DefaultInstructions,
		},
	}

	reader := bufio.NewReader(os.Stdin)
	for {
		logging.Log("> ")
		userMessage, err := reader.ReadString('\n')
		if err != nil {
			return fmt.Errorf("reading user prompt: %w", err)
		}
		userMessage = strings.ReplaceAll(userMessage, "\n", "")
		if userMessage == "" {
			continue
		}
		messages = append(messages, api.NewUserMessage(userMessage))

		for i := range MaxIterationsPerUserMessage {
			res, err := streamIteration(ctx, c, model, messages, tools, i)
			if err != nil {
				return fmt.Errorf("running streaming iteration: %w", err)
			}
			messages = append(messages, res.assistantMessage)
			if len(res.toolCalls) == 0 {
				break // the model is done with this user message
			}
			logging.Log("\n")
			for _, tc := range res.toolCalls {
				messages = append(messages, tools.ExecTool(ctx, tc))
			}
		}
	}
}

type iterationResult struct {
	assistantMessage api.Message
	toolCalls        []api.ToolCall
}

// streamIteration executes one turn of the agent loop: it streams a single chat
// completion, renders the response as it arrives, and reassembles any tool
// calls the model requested. The returned assistantMessage (which echoes the
// tool calls, as the API requires) should be appended to the conversation, and
// if toolCalls is non-empty the caller should execute them, append their
// results, and run another iteration.
func streamIteration(
	ctx context.Context,
	c api.Client,
	model string,
	messages []api.Message,
	tools Tools,
	iteration int,
) (iterationResult, error) {
	stream, err := c.ChatCompletionStream(ctx, api.ChatCompletionRequest{
		Model:    model,
		Messages: messages,
		Tools:    tools.GetDefinitions(),
		Stream:   true,
	})
	if err != nil {
		return iterationResult{}, fmt.Errorf("executing chat completion: %w", err)
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
			return iterationResult{}, fmt.Errorf("reading next message from stream: %w", err)
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
				logging.Log("\n")
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
		return iterationResult{}, fmt.Errorf("iteration %d: provider returned an empty response", iteration)
	}
	return iterationResult{
		assistantMessage: api.Message{
			Role:      api.RoleAssistant,
			Content:   agentResponse.String(),
			ToolCalls: calls,
		},
		toolCalls: calls,
	}, nil
}

func toolRegistry() (Tools, error) {
	tools := NewToolRegistry()
	if err := tools.RegisterTool("read", ReadTool()); err != nil {
		return nil, fmt.Errorf("registering tool read: %w", err)
	}
	if err := tools.RegisterTool("edit", EditTool()); err != nil {
		return nil, fmt.Errorf("registering tool edit: %w", err)
	}
	if err := tools.RegisterTool("bash", BashTool()); err != nil {
		return nil, fmt.Errorf("registering tool bash: %w", err)
	}
	if err := tools.RegisterTool("fetch", FetchTool()); err != nil {
		return nil, fmt.Errorf("registering tool fetch: %w", err)
	}
	return tools, nil
}
