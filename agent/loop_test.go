package agent_test

import (
	"context"
	"fmt"
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
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		chunks := []string{
			`{"choices":[{"delta":{"reasoning_content":"thinking..."}}]}`,
			`{"choices":[{"delta":{"content":"Hello"}}]}`,
			`{"choices":[{"delta":{"content":" world"},"finish_reason":"stop"}]}`,
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
	loop := agent.Loop{
		AgentID:       "root",
		Client:        api.NewClient(baseURL, "test-key"),
		Model:         "test-model",
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

	inputCh := make(chan agent.Input)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	done := make(chan error, 1)
	go func() {
		done <- loop.Run(ctx, inputCh)
	}()

	inputCh <- agent.Input{Kind: agent.InputKindUserMessage, Text: "hi"}
	// Give the turn a moment to complete, then end the session.
	time.Sleep(500 * time.Millisecond)
	close(inputCh)

	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("Run returned error: %v", err)
		}
	case <-ctx.Done():
		t.Fatal("Run did not return after input channel closed")
	}

	// The sender owns the channel: with the session over, close it and
	// wait for the renderer to drain.
	close(eventCh)
	<-drained

	// Verify the event sequence: awaiting input, user message, thinking
	// delta, content deltas, message end, awaiting input again.
	want := []events.Kind{
		events.KindAwaitingInput,
		events.KindUserMessage,
		events.KindThinkingDelta,
		events.KindContentDelta,
		events.KindContentDelta,
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
	if got[2].Text != "thinking..." || got[3].Text != "Hello" || got[4].Text != " world" {
		t.Errorf("unexpected delta texts: %q, %q, %q", got[2].Text, got[3].Text, got[4].Text)
	}
}

func kinds(evs []events.Event) []events.Kind {
	ks := make([]events.Kind, len(evs))
	for i, ev := range evs {
		ks[i] = ev.Kind
	}
	return ks
}
