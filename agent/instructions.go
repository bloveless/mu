package agent

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path"
	"strings"
	"time"
)

type facts struct {
	WorkingDirectory string
	Now              time.Time
}

// String returns an agent-readable string representation of the facts.
func (f facts) String() string {
	return fmt.Sprintf("Current working directory: %s\nConversation started at: %s", f.WorkingDirectory, f.Now)
}

// getFacts is a best effort function that returns some facts that about the current environment. If any fact
// couldn't be determined, the field will be left empty.
func getFacts() facts {
	wd, err := os.Getwd()
	if err != nil {
		wd = ""
	}
	return facts{
		WorkingDirectory: wd,
		Now:              time.Now().UTC(),
	}
}

// getAgentInstructions returns the agent's instructions for the current project.
func getAgentInstructions(ctx context.Context) (string, int) {
	// codex will start at the project root and concatenate AGENTS.md files down to the current working directory to
	// build up the instructions... this might be interesting eventually. https://learn.chatgpt.com/docs/agent-configuration/agents-md
	f, err := os.ReadFile("AGENTS.md")
	if err == nil {
		return string(f), len(f)
	}

	gitRoot, err := getGitRoot(ctx)
	if err != nil {
		return "", 0
	}
	f, err = os.ReadFile(path.Join(gitRoot, "AGENTS.md"))
	if err != nil {
		return "", 0
	}
	return string(f), len(f)
}

// getGitRoot returns the root directory of the current git repository.
func getGitRoot(ctx context.Context) (string, error) {
	ctx, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, "git", "rev-parse", "--show-toplevel")
	path, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("failed to get git root: %w", err)
	}
	return strings.TrimSpace(string(path)), nil
}
