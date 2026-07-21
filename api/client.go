package api

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/bloveless/mu/logging"
)

type Client struct {
	baseURL    *url.URL
	apiKey     string
	httpClient *http.Client
}

func NewClient(baseURL *url.URL, apiKey string) Client {
	return Client{
		baseURL: baseURL,
		apiKey:  apiKey,
		httpClient: &http.Client{
			Timeout: 5 * time.Minute,
		},
	}
}

type ChatStream struct {
	scanner   *bufio.Scanner
	resp      io.ReadCloser
	toolCalls *ToolCallAccumulator
}

// Recv iterates the underlying scanner forward until a message or an error is received
func (cs ChatStream) Recv() (StreamChunk, error) {
	for cs.scanner.Scan() {
		line := cs.scanner.Text()
		if !strings.HasPrefix(line, "data: ") {
			continue
		}
		data := strings.TrimPrefix(line, "data: ")
		if data == "[DONE]" {
			break
		}
		var chunk StreamChunk
		if err := json.Unmarshal([]byte(data), &chunk); err != nil {
			logging.Error("received malformed chunk [%s]: %s", data, err)
			continue // not sure what might be malformed json so log and continue
		}
		for _, choice := range chunk.Choices {
			cs.toolCalls.Add(choice.Delta.ToolCalls)
		}
		return chunk, nil
	}
	if err := cs.scanner.Err(); err != nil {
		return StreamChunk{}, fmt.Errorf("reading from chat stream scanner: %w", err)
	}
	return StreamChunk{}, io.EOF
}

// Close closes the ChatStream by closing the underlying response body
func (cs ChatStream) Close() error {
	return cs.resp.Close()
}

// ToolCalls returns the tool calls accumulated from every chunk the stream has
// received so far, ordered by stream index, or nil if the model made no tool
// calls. Call it after Recv has returned io.EOF to get the complete calls.
func (cs ChatStream) ToolCalls() []ToolCall {
	return cs.toolCalls.ToolCalls()
}

// ToolCallAccumulator reassembles tool calls that arrive fragmented across
// streamed chunks. OpenAI-compatible providers emit tool calls incrementally:
// the first fragment for a call carries the ID and function name while
// subsequent fragments (matched by Index) append to the JSON arguments.
type ToolCallAccumulator struct {
	calls map[int]*ToolCall
	order []int
}

func NewToolCallAccumulator() *ToolCallAccumulator {
	return &ToolCallAccumulator{calls: make(map[int]*ToolCall)}
}

// Add merges a chunk's tool-call fragments into the accumulated calls.
func (a *ToolCallAccumulator) Add(fragments []StreamToolCall) {
	for _, f := range fragments {
		tc, ok := a.calls[f.Index]
		if !ok {
			tc = &ToolCall{}
			a.calls[f.Index] = tc
			a.order = append(a.order, f.Index)
		}
		if f.ID != "" {
			tc.ID = f.ID
		}
		if f.CallType != "" {
			tc.Type = f.CallType
		}
		if f.Function.Name != "" {
			tc.Function.Name = f.Function.Name
		}
		tc.Function.Arguments += f.Function.Arguments
	}
}

// ToolCalls returns the accumulated tool calls ordered by stream index, or nil
// if the model made no tool calls.
func (a *ToolCallAccumulator) ToolCalls() []ToolCall {
	if len(a.order) == 0 {
		return nil
	}
	calls := make([]ToolCall, 0, len(a.order))
	for _, idx := range a.order {
		calls = append(calls, *a.calls[idx])
	}
	return calls
}

// ChatCompletion issues a chat completion request to an openAI compatible endpoint
func (c Client) ChatCompletionStream(ctx context.Context, req ChatCompletionRequest) (_ ChatStream, retErr error) {
	reqURL := c.baseURL.JoinPath("/chat/completions")
	body, err := json.Marshal(req)
	if err != nil {
		return ChatStream{}, fmt.Errorf("marshalling request: %w", err)
	}
	var reqBytes bytes.Buffer
	tee := io.TeeReader(bytes.NewReader(body), &reqBytes)
	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, reqURL.String(), tee)
	if err != nil {
		return ChatStream{}, fmt.Errorf("creating request with context: %w", err)
	}
	if c.apiKey != "" {
		httpReq.Header.Set("Authorization", "Bearer: "+c.apiKey)
	}
	httpReq.Header.Set("User-Agent", "mu2/0.1.0")
	httpReq.Header.Set("Content-Type", "application/json")
	httpReq.Header.Set("Accept", "application/json")
	httpResp, err := c.httpClient.Do(httpReq) //nolint:bodyclose // intentionally leaving body open so it can be used in the ChatStream
	if err != nil {
		return ChatStream{}, fmt.Errorf("executing http request: %w", err)
	}
	if httpResp.StatusCode < 200 || httpResp.StatusCode > 299 {
		errorMsg, err := io.ReadAll(httpResp.Body)
		if err != nil {
			return ChatStream{}, fmt.Errorf("reading error body: %w", err)
		}
		var prettyJSON bytes.Buffer
		err = json.Indent(&prettyJSON, reqBytes.Bytes(), "", "  ")
		switch err {
		case nil:
			logging.Debug("failed request body:\n%s\n", prettyJSON.String())
		default:
			logging.Debug("failed request body: %s\n", reqBytes.String())
		}
		return ChatStream{}, fmt.Errorf("request failed [%s; code: %d]: %s response: %s", reqURL.String(), httpResp.StatusCode, errorMsg, reqBytes.String())
	}
	scanner := bufio.NewScanner(httpResp.Body)
	// claude suggested this but I'd prefer to roll with the defaults for now. I wanted to document this in case I run
	// into a long line issue in the future
	// scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024) // handle long lines
	return ChatStream{
		resp:      httpResp.Body,
		scanner:   scanner,
		toolCalls: NewToolCallAccumulator(),
	}, nil
}
