package main

import (
	"bufio"
	"context"
	_ "embed"
	"flag"
	"fmt"
	"net/url"
	"os"
	"os/signal"
	"strings"
	"syscall"

	"golang.org/x/sync/errgroup"

	"github.com/bloveless/mu/agent"
	"github.com/bloveless/mu/api"
	"github.com/bloveless/mu/events"
	"github.com/bloveless/mu/logging"
	"github.com/bloveless/mu/render"
	"github.com/bloveless/mu/tools"
)

//go:embed DEFAULT_INSTRUCTIONS.md
var DefaultInstructions string

//go:embed AGENTS.md
var AgentInstructions string

func main() {
	verbose := flag.Bool("v", false, "enable debug logging")
	provider := flag.String("provider", "opencode-go", "provider to use")
	model := flag.String("model", "deepseek-v4-pro", "model to use")
	maxIterations := flag.Int("max-iterations", 50, "maximum number of iterations per user message")
	flag.Parse()

	if err := run(*verbose, *provider, *model, *maxIterations); err != nil {
		logging.Error("error running mu: %s\n", err)
		os.Exit(1)
	}
}

// run starts the agent CLI with the selected provider and model, processing standard input and rendering agent events.
// It returns an error if provider configuration, tool setup, agent execution, or pipeline coordination fails.
func run(verbose bool, provider, model string, maxIterations int) error {
	logging.SetVerbose(verbose)
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	providers, err := api.GetProviders(ctx)
	if err != nil {
		return fmt.Errorf("refreshing models: %w", err)
	}
	p, ok := providers[provider]
	if !ok {
		return fmt.Errorf("unable to find provider in providers.json")
	}
	baseURL, err := url.Parse(p.API)
	if err != nil {
		return fmt.Errorf("parsing base URL: %w", err)
	}
	if len(p.Env) != 1 {
		return fmt.Errorf(
			"provider didn't have exactly one environment variable in models.dev... not quite sure what to do with that",
		)
	}
	apiKey := os.Getenv(p.Env[0])
	if apiKey == "" {
		return fmt.Errorf("unable to find required API key [%s] in environment", p.Env[0])
	}
	c := api.NewClient(baseURL, apiKey)

	m, ok := p.Models[model]
	if !ok {
		return fmt.Errorf("model %q not found in provider %q", model, provider)
	}

	mainToolsReg, err := mainAgentToolRegistry()
	if err != nil {
		return fmt.Errorf("getting tool registry: %w", err)
	}

	// The pipeline: stdin adapter -|inputCh|-> agent -|eventCh|-> renderer.
	// Each stage runs in its own goroutine and knows nothing about the
	// others' medium, so the CLI adapters can later be swapped for Bubble
	// Tea or JSON-RPC without touching the agent.
	g, ctx := errgroup.WithContext(ctx)
	inputCh := make(chan agent.Input)
	g.Go(func() error {
		defer close(inputCh)
		readStdinInputs(ctx, inputCh)
		return nil
	})
	eventCh := make(chan events.Event, 64)
	g.Go(func() error {
		defer close(eventCh)
		loop := agent.Loop{
			AgentID:           "root",
			Client:            c,
			MaxIterations:     maxIterations,
			Model:             m,
			Provider:          p,
			ToolsRegistry:     mainToolsReg,
			SystemPrompt:      DefaultInstructions,
			AgentInstructions: AgentInstructions,
			Events:            eventCh,
		}
		if err := loop.Run(ctx, inputCh); err != nil {
			return fmt.Errorf("running agent loop: %w", err)
		}
		return nil
	})
	g.Go(func() error {
		renderer := render.NewTerminal(fmt.Sprintf("%s:%s > ", p.ID, model))
		for ev := range eventCh {
			renderer.Handle(ev)
		}
		logging.Log("\n")
		return nil
	})

	return g.Wait()
}

// readStdinInputs adapts stdin lines into agent inputs, one per line. It
// returns when stdin closes (e.g. Ctrl-D) or ctx is cancelled. A blocking
// stdin read can't be interrupted, so on cancellation the reader goroutine
// may leak until the process exits; that's acceptable since the CLI is
// shutting down anyway.
func readStdinInputs(ctx context.Context, out chan<- agent.Input) {
	lines := make(chan string)
	go func() {
		defer close(lines)
		reader := bufio.NewReader(os.Stdin)
		for {
			line, err := reader.ReadString('\n')
			if line != "" {
				select {
				case lines <- strings.TrimSpace(line):
				case <-ctx.Done():
					return
				}
			}
			if err != nil {
				return
			}
		}
	}()
	for {
		select {
		case <-ctx.Done():
			return
		case line, ok := <-lines:
			if !ok {
				return
			}
			select {
			case <-ctx.Done():
				return
			case out <- agent.Input{Kind: agent.InputKindUserMessage, Text: line}:
			}
		}
	}
}

func mainAgentToolRegistry() (tools.Registry, error) {
	tr := tools.NewRegistry()
	if err := tr.Register("read", tools.Read()); err != nil {
		return nil, fmt.Errorf("registering tool read: %w", err)
	}
	if err := tr.Register("edit", tools.Edit()); err != nil {
		return nil, fmt.Errorf("registering tool edit: %w", err)
	}
	if err := tr.Register("bash", tools.Bash()); err != nil {
		return nil, fmt.Errorf("registering tool bash: %w", err)
	}
	if err := tr.Register("fetch", tools.Fetch()); err != nil {
		return nil, fmt.Errorf("registering tool fetch: %w", err)
	}
	return tr, nil
}
