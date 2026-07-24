package tools

import (
	"context"
	"strings"
	"testing"

	"github.com/bloveless/mu/api"
	"github.com/bloveless/mu/events"
)

func mockEmitter(ctx context.Context, kind events.Kind, message string) {}

func bashCall(t *testing.T, args string) api.Message {
	t.Helper()
	tool := Bash()
	return tool.Exec(context.Background(), api.ToolCall{
		ID: "test-call",
		Function: api.FunctionCall{
			Name:      "bash",
			Arguments: args,
		},
	}, mockEmitter)
}

func TestBashToolRunsCommandAndReturnsOutput(t *testing.T) {
	msg := bashCall(t, `{"command": "echo hello"}`)
	if !strings.Contains(msg.Content, "hello") {
		t.Errorf("expected stdout to contain %q, got: %q", "hello", msg.Content)
	}
	if !strings.Contains(msg.Content, "exit code: 0") {
		t.Errorf("expected exit code 0, got: %q", msg.Content)
	}
}

func TestBashToolReportsNonZeroExitWithStderr(t *testing.T) {
	msg := bashCall(t, `{"command": "echo oops >&2; exit 3"}`)
	if !strings.Contains(msg.Content, "exit code: 3") {
		t.Errorf("expected exit code 3, got: %q", msg.Content)
	}
	if !strings.Contains(msg.Content, "oops") {
		t.Errorf("expected stderr to be included in result, got: %q", msg.Content)
	}
	if strings.Contains(msg.Content, "failed to start") {
		t.Errorf("non-zero exit should not be reported as a start failure, got: %q", msg.Content)
	}
}
