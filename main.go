package main

import (
	"bufio"
	"context"
	_ "embed"
	"errors"
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

//go:embed DEFAULT_INSTRUCTIONS_SUBAGENT.md
var DefaultInstructionsSubagent string

func main() {
	verbose := flag.Bool("v", false, "enable debug logging")
	provider := flag.String("provider", "opencode-go", "provider to use")
	model := flag.String("model", "deepseek-v4-pro", "model to use")
	maxIterations := flag.Int("max-iterations", 50, "maximum number of iterations per user message")
	subagentProvider := flag.String("subagent-provider", "opencode", "provider for sub-agent model")
	subagentModel := flag.String("subagent-model", "deepseek-v4-flash-free", "model for sub-agent")
	flag.Parse()

	if err := run(*verbose, *provider, *model, *subagentProvider, *subagentModel, *maxIterations); err != nil {
		logging.Error("error running mu: %s\n", err)
		os.Exit(1)
	}
}

// run starts the agent CLI with the selected provider and model, processing standard input and rendering agent events.
// It returns an error if provider configuration, tool setup, agent execution, or pipeline coordination fails.
func run(verbose bool, provider, model, subagentProvider, subagentModel string, maxIterations int) error {
	logging.SetVerbose(verbose)
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	providers, err := api.GetProviders(ctx)
	if err != nil {
		return fmt.Errorf("refreshing models: %w", err)
	}

	// The pipeline: stdin adapter -|inputCh|-> agent -|eventCh|-> renderer.
	// Each stage runs in its own goroutine and knows nothing about the
	// others' medium, so the CLI adapters can later be swapped for Bubble
	// Tea or JSON-RPC without touching the agent.
	wg, ctx := errgroup.WithContext(ctx)
	inputCh := make(chan string, 1)
	wg.Go(func() error {
		defer close(inputCh)
		readStdinInputs(ctx, inputCh)
		return nil
	})
	eventCh := make(chan events.Event, 1)
	wg.Go(func() error {
		defer close(eventCh)
		subToolsReg, err := toolRegistry(nil)
		if err != nil {
			return fmt.Errorf("getting sub-agent tool registry: %w", err)
		}
		subAgent, err := getAgent(eventCh, providers, subagentProvider, subagentModel, maxIterations, subToolsReg, "subagent", DefaultInstructionsSubagent)
		if err != nil {
			return fmt.Errorf("get sub-agent: %w", err)
		}
		subagentTool := agent.SubagentTool(subAgent)
		mainToolsReg, err := toolRegistry(subagentTool)
		if err != nil {
			return fmt.Errorf("getting tool registry: %w", err)
		}
		a, err := getAgent(eventCh, providers, provider, model, maxIterations, mainToolsReg, "root", DefaultInstructions)
		if err != nil {
			return fmt.Errorf("get agent: %w", err)
		}
		session := a.NewSession(ctx)
		a.Emit(ctx, events.KindAwaitingInput, "")
		for input := range inputCh {
			if err := session.ExecutePrompt(ctx, input); err != nil && !errors.Is(err, agent.ErrMaxIterationsReached) {
				return fmt.Errorf("running agent loop: %w", err)
			}
			a.Emit(ctx, events.KindMessageEnd, "")
			a.Emit(ctx, events.KindAwaitingInput, "")
		}
		return nil
	})
	wg.Go(func() error {
		renderer := render.NewTerminal(fmt.Sprintf("%s:%s > ", provider, model))
		for ev := range eventCh {
			renderer.Handle(ev)
		}
		logging.Log("\n")
		return nil
	})

	return wg.Wait()
}

func getAgent(e chan<- events.Event, providers api.Providers, provider, model string, maxIterations int, toolsReg tools.Registry, id, systemPrompt string) (*agent.Agent, error) {
	p, ok := providers[provider]
	if !ok {
		return nil, fmt.Errorf("provider %q not found in providers.json", provider)
	}
	baseURL, err := url.Parse(p.API)
	if err != nil {
		return nil, fmt.Errorf("parsing base URL: %w", err)
	}
	if len(p.Env) != 1 {
		return nil, fmt.Errorf("provider didn't have exactly one environment variable in models.dev")
	}
	apiKey := os.Getenv(p.Env[0])
	if apiKey == "" {
		return nil, fmt.Errorf("unable to find required API key [%s] in environment", p.Env[0])
	}
	m, ok := p.Models[model]
	if !ok {
		return nil, fmt.Errorf("model %q not found in provider %q", model, provider)
	}
	return &agent.Agent{
		ID:            id,
		Client:        api.NewClient(baseURL, apiKey),
		MaxIterations: maxIterations,
		Model:         m,
		Provider:      p,
		ToolsRegistry: toolsReg,
		SystemPrompt:  systemPrompt,
		Events:        e,
	}, nil
}

// readStdinInputs adapts stdin lines into agent inputs, one per line. It
// returns when stdin closes (e.g. Ctrl-D) or ctx is cancelled. A blocking
// stdin read can't be interrupted, so on cancellation the reader goroutine
// may leak until the process exits; that's acceptable since the CLI is
// shutting down anyway.
func readStdinInputs(ctx context.Context, out chan<- string) {
	lines := make(chan string)
	go func() {
		defer close(lines)
		reader := bufio.NewReader(os.Stdin)
		for {
			line, err := reader.ReadString('\n')
			line = strings.TrimSpace(line)
			if line != "" {
				select {
				case lines <- line:
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
			case out <- line:
			}
		}
	}
}

func toolRegistry(subagentTool *tools.Tool) (tools.Registry, error) {
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
	if subagentTool != nil {
		if err := tr.Register("subagent", subagentTool); err != nil {
			return nil, fmt.Errorf("registering tool subagent: %w", err)
		}
	}
	return tr, nil
}
