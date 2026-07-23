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

	"github.com/bloveless/mu/agent"
	"github.com/bloveless/mu/api"
	"github.com/bloveless/mu/logging"
	"github.com/bloveless/mu/tools"
)

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
	logging.SetVerbose(verbose)
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
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
	mainToolsReg, err := mainAgentToolRegistry()
	if err != nil {
		return fmt.Errorf("getting tool registry: %w", err)
	}

	messages := []api.Message{
		{
			Role:    api.RoleSystem,
			Content: DefaultInstructions,
		},
	}

	loop := agent.Loop{
		Client: c,
		Model:  model,
		Tools:  mainToolsReg,
	}

	reader := bufio.NewReader(os.Stdin)
	for {
		logging.Log("%s:%s > ", provider.Name, model)
		userMessage, err := reader.ReadString('\n')
		if err != nil {
			return fmt.Errorf("reading user prompt: %w", err)
		}
		userMessage = strings.ReplaceAll(userMessage, "\n", "")
		if userMessage == "" {
			continue
		}
		logging.Log("\n\n")
		messages, err = loop.Run(ctx, append(messages, api.NewUserMessage(userMessage)))
		if err != nil {
			return fmt.Errorf("running agent loop: %w", err)
		}
		logging.Log("\n")
		logging.Debug("new messages generated: %d", len(messages))
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
