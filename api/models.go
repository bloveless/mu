package api

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"time"
)

// Providers is the full /api.json response: provider ID -> Provider.
type Providers map[string]Provider

// Provider describes an AI SDK provider and the models it serves.
type Provider struct {
	ID     string                   `json:"id"`            // directory name, e.g. "opencode"
	Name   string                   `json:"name"`          // display name, e.g. "OpenCode Zen"
	NPM    string                   `json:"npm"`           // AI SDK package, e.g. "@ai-sdk/openai-compatible"
	Env    []string                 `json:"env"`           // env vars used for auth, e.g. ["OPENCODE_API_KEY"]
	API    string                   `json:"api,omitempty"` // base URL, required for openai-compatible providers
	Doc    string                   `json:"doc,omitempty"` // link to provider docs
	Models map[string]ProviderModel `json:"models"`        // model ID -> Model, keyed by AI SDK model ID
}

// ProviderModel describes a single model's capabilities, cost, and limits.
type ProviderModel struct {
	ID          string      `json:"id"`
	Name        string      `json:"name"`
	Family      string      `json:"family,omitempty"`      // e.g. "gpt-codex"
	Attachment  bool        `json:"attachment"`            // supports file/image attachments
	Reasoning   bool        `json:"reasoning"`             // supports chain-of-thought / reasoning
	Interleaved Interleaved `json:"interleaved,omitempty"` // bool or object; see custom unmarshal
	ToolCall    bool        `json:"tool_call"`
	Temperature bool        `json:"temperature"` // supports temperature param
	OpenWeights bool        `json:"open_weights"`
	Knowledge   string      `json:"knowledge,omitempty"`    // knowledge cutoff, e.g. "2024-10"
	ReleaseDate string      `json:"release_date,omitempty"` // e.g. "2025-11-13"
	LastUpdated string      `json:"last_updated,omitempty"`

	Modalities Modalities `json:"modalities"`
	Cost       Cost       `json:"cost"`
	Limit      Limit      `json:"limit"`
}

// Modalities lists the supported input/output content types, e.g.
// {"input": ["text","image"], "output": ["text"]}.
type Modalities struct {
	Input  []string `json:"input"`
	Output []string `json:"output"`
}

// Cost holds per-million-token USD pricing. Free models report zero values.
type Cost struct {
	Input       float64 `json:"input"`
	Output      float64 `json:"output"`
	CacheRead   float64 `json:"cache_read,omitempty"`
	CacheWrite  float64 `json:"cache_write,omitempty"`
	InputAudio  float64 `json:"input_audio,omitempty"`
	OutputAudio float64 `json:"output_audio,omitempty"`
}

// Limit holds the model's context window and max-output token limits — this
// is the field opencode/@opencode-ai/sdk reads for "how much context is left".
type Limit struct {
	Context int `json:"context"` // total context window, in tokens
	Output  int `json:"output"`  // max tokens the model can generate per response
}

// Interleaved represents the "interleaved" field, which in the source TOML/
// schema is either a plain bool (general support) or an object specifying
// the reasoning-content placement format, e.g. {"field": "reasoning_content"}.
type Interleaved struct {
	Supported bool   `json:"-"`
	Field     string `json:"field,omitempty"`
}

func (i *Interleaved) UnmarshalJSON(data []byte) error {
	var asBool bool
	if err := json.Unmarshal(data, &asBool); err == nil {
		i.Supported = asBool
		return nil
	}
	var asObj struct {
		Field string `json:"field"`
	}
	if err := json.Unmarshal(data, &asObj); err != nil {
		return err
	}
	i.Supported = true
	i.Field = asObj.Field
	return nil
}

func GetProviders(ctx context.Context) (_ Providers, retErr error) {
	fi, err := os.Stat("providers.json")
	switch {
	case err != nil:
		// error getting file status or file doesn't exist, make a new one
	case time.Since(fi.ModTime()) > 24*time.Hour:
		// file is older than 24 hours, make a new one
	default:
		if p, err := loadProvidersFile("providers.json"); err == nil {
			fmt.Fprintf(os.Stderr, "loaded providers.json file\n")
			// return fresh parsed providers file
			return p, nil
		}
		// unable to load providers file, make a new one
	}
	client := http.Client{
		Timeout: 15 * time.Second,
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, "https://models.dev/api.json", http.NoBody)
	if err != nil {
		return nil, fmt.Errorf("creating request to models.dev: %w", err)
	}
	resp, err := client.Do(req)
	if err != nil {
		return nil, fmt.Errorf("getting models.dev/api.json: %w", err)
	}
	defer func() {
		if err := resp.Body.Close(); err != nil && retErr == nil {
			retErr = fmt.Errorf("closing models.dev body: %w", err)
		}
	}()
	var p Providers
	file, err := os.Create("providers.json")
	if err != nil {
		return nil, fmt.Errorf("creating provider.json file: %w", err)
	}
	defer func() {
		if err := file.Close(); err != nil && retErr == nil {
			retErr = fmt.Errorf("closing providers.json file: %w", err)
		}
	}()
	tee := io.TeeReader(resp.Body, file)
	if err := json.NewDecoder(tee).Decode(&p); err != nil {
		return nil, fmt.Errorf("decoding providers from models.dev: %w", err)
	}
	return p, nil
}

func loadProvidersFile(file string) (Providers, error) {
	f, err := os.Open(file)
	if err != nil {
		return nil, fmt.Errorf("reading providers file: %w", err)
	}
	defer f.Close()
	var p Providers
	if err := json.NewDecoder(f).Decode(&p); err != nil {
		return nil, fmt.Errorf("decoding providers file: %w", err)
	}
	return p, nil
}
