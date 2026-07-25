package agent_test

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/bloveless/mu/agent"
	"github.com/bloveless/mu/api"
	"github.com/bloveless/mu/events"
	"github.com/bloveless/mu/tools"
)

func TestSubagentToolRunsChildLoopAndReturnsResult(t *testing.T) {
	// Set up a fake HTTP server that the sub-agent's client will call.
	var mu sync.Mutex
	var subagentReqBody []byte
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var err error
		body, err := io.ReadAll(r.Body)
		mu.Lock()
		subagentReqBody = body
		mu.Unlock()
		if err != nil {
			t.Errorf("reading subagent request body: %v", err)
		}
		defer r.Body.Close()
		w.Header().Set("Content-Type", "text/event-stream")
		chunks := []string{
			`{"choices":[{"delta":{"content":"I investigated the codebase."}}]}`,
			`{"choices":[{"delta":{"content":" Here is my summary."},"finish_reason":"stop"}]}`,
		}
		for _, c := range chunks {
			fmt.Fprintf(w, "data: %s\n\n", c)
		}
		fmt.Fprint(w, "data: [DONE]\n\n")
	}))
	defer server.Close()

	baseURL, err := url.Parse(server.URL)
	if err != nil {
		t.Fatalf("parsing test server URL: %v", err)
	}

	subagentClient := api.NewClient(baseURL, "test-key")
	subagentModel := api.ProviderModel{ID: "test-model"}
	subagentProvider := api.Provider{ID: "test-provider"}

	// The sub-agent's event channel — events are collected for validation.
	subEventCh := make(chan events.Event, 64)
	subDone := make(chan struct{})

	var subEvents []events.Event
	go func() {
		defer close(subDone)
		for e := range subEventCh {
			subEvents = append(subEvents, e)
		}
	}()

	tool := agent.SubagentTool(&agent.Agent{
		ID:            "subagent",
		Client:        subagentClient,
		MaxIterations: 50,
		Model:         subagentModel,
		Provider:      subagentProvider,
		ToolsRegistry: tools.NewRegistry(), // no tools needed for this test
		SystemPrompt:  "you are a sub-agent",
		Events:        subEventCh,
	})

	// Collect emitted events.
	var emitted []events.Event
	emit := func(ctx context.Context, kind events.Kind, text string) {
		emitted = append(emitted, events.Event{AgentID: "subagent", Kind: kind, Text: text})
	}

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	msg := tool.Exec(ctx, api.ToolCall{
		ID: "test-subagent-call",
		Function: api.FunctionCall{
			Name:      "subagent",
			Arguments: `{"prompt": "Investigate the project structure"}`,
		},
	}, emit)

	// Close the sub-agent's event channel and wait for the collector to finish.
	close(subEventCh)
	<-subDone

	// Verify tool result contains the sub-agent's response.
	if !strings.Contains(msg.Content, "I investigated the codebase. Here is my summary.") {
		t.Errorf("expected sub-agent response in tool result, got: %q", msg.Content)
	}
	if msg.Role != api.RoleTool {
		t.Errorf("expected RoleTool, got %s", msg.Role)
	}

	// Verify the tool emitted a progress event.
	if len(emitted) != 1 || emitted[0].Kind != events.KindToolProgress {
		t.Errorf("expected one KindToolProgress event, got %d events: %+v", len(emitted), emitted)
	}

	// Verify the sub-agent emitted events on its own channel using its AgentID.
	if len(subEvents) == 0 {
		t.Error("expected sub-agent events on its own event channel, got none")
	}
	for _, e := range subEvents {
		if e.AgentID != "subagent" {
			t.Errorf("expected sub-agent event AgentID %q, got %q", "subagent", e.AgentID)
		}
	}

	// Verify the sub-agent's request body includes the right model.
	mu.Lock()
	body := subagentReqBody
	mu.Unlock()
	if body == nil {
		t.Fatal("sub-agent request body was not captured")
	}
	if !strings.Contains(string(body), "test-model") {
		t.Errorf("expected request to use sub-agent model, got: %s", string(body))
	}

	// Verify the tool registry passed to the sub-agent does NOT include "subagent".
	if strings.Contains(string(body), `"name":"subagent"`) {
		t.Error("sub-agent should not have access to the subagent tool")
	}
}

func TestSubagentToolMissingPrompt(t *testing.T) {
	tool := agent.SubagentTool(&agent.Agent{})
	msg := tool.Exec(context.Background(), api.ToolCall{
		ID:       "test-call",
		Function: api.FunctionCall{Name: "subagent", Arguments: `{}`},
	}, func(ctx context.Context, kind events.Kind, text string) {})
	if !strings.Contains(msg.Content, "missing required argument: prompt") {
		t.Errorf("expected missing prompt error, got: %q", msg.Content)
	}
}
