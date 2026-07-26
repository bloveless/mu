package tui

import "testing"

func TestRunHandle(t *testing.T) {
	r := &RunHandle{}
	if r.Cancel() {
		t.Fatal("idle handle should report no cancellation")
	}
	cancelled := false
	r.Set(func() { cancelled = true })
	if !r.Cancel() {
		t.Fatal("expected cancellation reported")
	}
	if !cancelled {
		t.Fatal("expected cancel func called")
	}
	r.Clear()
	if r.Cancel() {
		t.Fatal("cleared handle should report no cancellation")
	}
}
