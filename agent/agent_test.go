package agent_test

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"testing"
	"time"

	"github.com/bloveless/mu/agent"
	"github.com/bloveless/mu/api"
	"github.com/bloveless/mu/events"
	"github.com/bloveless/mu/tools"
)

// TestPipelineSmoke runs the agent session against a fake streaming
// provider and records the emitted events, verifying the session consumes
// an input, runs a turn, and ends when the input channel closes.
func TestPipelineSmoke(t *testing.T) {
	var reqBody []byte
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var err error
		reqBody, err = io.ReadAll(r.Body)
		if err != nil {
			t.Errorf("reading request body: %v", err)
		}
		defer r.Body.Close()
		w.Header().Set("Content-Type", "text/event-stream")
		chunks := []string{
			`{"choices":[{"delta":{"reasoning_content":"thinking..."}}]}`,
			`{"choices":[{"delta":{"content":"Hello"}}]}`,
			`{"choices":[{"delta":{"content":" world"},"finish_reason":"stop"}]}`,
			`{"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}`,
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

	eventCh := make(chan events.Event, 64)
	a := agent.Agent{
		ID:            "root",
		Client:        api.NewClient(baseURL, "test-key"),
		MaxIterations: 50,
		Model:         api.ProviderModel{},
		ToolsRegistry: tools.NewRegistry(),
		SystemPrompt:  "you are a test",
		Events:        eventCh,
	}

	// Drain the event channel the way a renderer goroutine would.
	var got []events.Event
	drained := make(chan struct{})
	go func() {
		defer close(drained)
		for ev := range eventCh {
			got = append(got, ev)
		}
	}()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	// Emit the initial awaiting-input event (mirrors main.go pattern).
	a.Emit(ctx, events.KindAwaitingInput, "")

	// Run the agent with a single prompt.
	session := a.NewSession(ctx)
	if err := session.ExecutePrompt(ctx, "hi"); err != nil {
		t.Fatalf("ExecutePrompt returned error: %v", err)
	}

	// Emit the post-turn events (mirrors main.go pattern).
	a.Emit(ctx, events.KindMessageEnd, "")
	a.Emit(ctx, events.KindAwaitingInput, "")

	// Close the event channel so the drainer finishes.
	close(eventCh)
	<-drained

	// Verify the event sequence: awaiting input, thinking delta, content
	// deltas, usage, message end, awaiting input again.
	want := []events.Kind{
		events.KindAwaitingInput,
		events.KindThinkingDelta,
		events.KindContentDelta,
		events.KindContentDelta,
		events.KindUsage,
		events.KindMessageEnd,
		events.KindAwaitingInput,
	}
	if len(got) != len(want) {
		t.Fatalf("got %d events %v, want %d %v", len(got), kinds(got), len(want), want)
	}
	for i, k := range want {
		if got[i].Kind != k {
			t.Errorf("event %d: got kind %v, want %v", i, got[i].Kind, k)
		}
		if got[i].AgentID != "root" {
			t.Errorf("event %d: got AgentID %q, want %q", i, got[i].AgentID, "root")
		}
	}
	if got[1].Text != "thinking..." || got[2].Text != "Hello" || got[3].Text != " world" {
		t.Errorf("unexpected delta texts: %q, %q, %q", got[1].Text, got[2].Text, got[3].Text)
	}
	if got[4].Text != "↑10 ↓5 R0 W0 CH0.0% $0.000 0.0%/0" {
		t.Errorf("unexpected usage text: %q", got[4].Text)
	}

	// Verify stream_options.include_usage is true in the outgoing request.
	var req api.ChatCompletionRequest
	if err := json.Unmarshal(reqBody, &req); err != nil {
		t.Fatalf("unmarshalling request body: %v", err)
	}
	if !req.StreamOptions.IncludeUsage {
		t.Error("stream_options.include_usage should be true")
	}
}

// TestReasoningOnlyStreamSucceeds verifies that a provider stream containing
// only reasoning_content (zero content, zero tool calls) with a finish
// reason does not crash the agent; it is treated as an empty assistant
// response that ends the turn normally.
func TestReasoningOnlyStreamSucceeds(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		defer r.Body.Close()
		w.Header().Set("Content-Type", "text/event-stream")
		chunks := []string{
			`{"choices":[{"delta":{"reasoning_content":"thinking..."}}]}`,
			`{"choices":[{"delta":{"reasoning_content":" more thinking"},"finish_reason":"stop"}]}`,
			`{"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}`,
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

	eventCh := make(chan events.Event, 64)
	a := agent.Agent{
		ID:            "root",
		Client:        api.NewClient(baseURL, "test-key"),
		MaxIterations: 50,
		Model:         api.ProviderModel{},
		ToolsRegistry: tools.NewRegistry(),
		SystemPrompt:  "you are a test",
		Events:        eventCh,
	}

	var got []events.Event
	drained := make(chan struct{})
	go func() {
		defer close(drained)
		for ev := range eventCh {
			got = append(got, ev)
		}
	}()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	session := a.NewSession(ctx)
	if err := session.ExecutePrompt(ctx, "hi"); err != nil {
		t.Fatalf("ExecutePrompt returned error on reasoning-only stream: %v", err)
	}

	close(eventCh)
	<-drained

	// Verify: thinking deltas and usage arrived; no content deltas, no
	// error event.
	if len(got) != 3 {
		t.Fatalf("got %d events %v, want 3 (2\u00d7 thinking + usage)", len(got), kinds(got))
	}
	if got[0].Kind != events.KindThinkingDelta || got[1].Kind != events.KindThinkingDelta {
		t.Errorf("first two events should be thinking deltas, got %v %v", got[0].Kind, got[1].Kind)
	}
	if got[2].Kind != events.KindUsage {
		t.Errorf("last event should be usage, got %v", got[2].Kind)
	}
}

func kinds(evs []events.Event) []events.Kind {
	ks := make([]events.Kind, len(evs))
	for i, ev := range evs {
		ks[i] = ev.Kind
	}
	return ks
}

// TestExecutePrompt_MaxIterationsExhausted verifies that when the model
// continuously requests tool calls, ExecutePrompt eventually hits the
// iteration cap, emits a KindError with the truncation message, and
// returns ErrMaxIterationsReached.
func TestExecutePrompt_MaxIterationsExhausted(t *testing.T) {
	// Server that always responds with a tool call — no content, no stop.
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		chunks := []string{
			`{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"ping","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}`,
			`{"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}`,
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

	// Register a dummy tool so the tool call resolves.
	tr := tools.NewRegistry()
	tr.Register("ping", &tools.Tool{
		Definition: api.ToolDefinition{
			Type: "function",
			Function: api.FunctionDefinition{
				Name:        "ping",
				Description: "A simple ping tool",
			},
		},
		Exec: func(ctx context.Context, tc api.ToolCall, emit tools.Emitter) api.Message {
			return api.NewToolResultMessage(tc.ID, "pong")
		},
	})

	eventCh := make(chan events.Event, 64)
	a := agent.Agent{
		ID:            "root",
		Client:        api.NewClient(baseURL, "test-key"),
		MaxIterations: 3,
		Model:         api.ProviderModel{},
		ToolsRegistry: tr,
		SystemPrompt:  "you are a test",
		Events:        eventCh,
	}

	// Drain the event channel the way a renderer goroutine would.
	var got []events.Event
	drained := make(chan struct{})
	go func() {
		defer close(drained)
		for ev := range eventCh {
			got = append(got, ev)
		}
	}()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	a.Emit(ctx, events.KindAwaitingInput, "")

	session := a.NewSession(ctx)
	if err := session.ExecutePrompt(ctx, "hi"); !errors.Is(err, agent.ErrMaxIterationsReached) {
		t.Fatalf("ExecutePrompt returned %v, want ErrMaxIterationsReached", err)
	}

	a.Emit(ctx, events.KindMessageEnd, "")
	a.Emit(ctx, events.KindAwaitingInput, "")

	close(eventCh)
	<-drained

	// Assert a KindError with the expected truncation message is present.
	var foundError bool
	wantMsg := "reached iteration cap (3) with tool calls still pending; turn truncated — the conversation is intact, send another message to continue"
	for _, ev := range got {
		if ev.Kind == events.KindError {
			foundError = true
			if ev.Text != wantMsg {
				t.Errorf("error message = %q, want %q", ev.Text, wantMsg)
			}
			if ev.AgentID != "root" {
				t.Errorf("error AgentID = %q, want %q", ev.AgentID, "root")
			}
		}
	}
	if !foundError {
		t.Error("expected KindError event not found in emitted events")
	}

	// Verify that all three iterations ran: each produced a tool result.
	var toolResults int
	for _, ev := range got {
		if ev.Kind == events.KindToolResult {
			toolResults++
		}
	}
	if toolResults != 3 {
		t.Errorf("got %d KindToolResult events, want 3 (one per iteration)", toolResults)
	}
}
