package tui

import (
	"context"
	"sync"
)

// RunHandle holds the cancel function for the agent turn currently in
// flight so the TUI can abort it (ctrl+c) without quitting the program.
// The zero value is ready to use.
type RunHandle struct {
	mu     sync.Mutex
	cancel context.CancelFunc
}

// Set records the cancel function for the in-flight run.
func (r *RunHandle) Set(cancel context.CancelFunc) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.cancel = cancel
}

// Clear records that no run is in flight.
func (r *RunHandle) Clear() {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.cancel = nil
}

// Cancel aborts the in-flight run, if any, and reports whether one was
// cancelled.
func (r *RunHandle) Cancel() bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.cancel == nil {
		return false
	}
	r.cancel()
	return true
}
