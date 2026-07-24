package api

import (
	"encoding/json"
	"time"
)

type Role string

const (
	RoleUser      Role = "user"
	RoleSystem    Role = "system"
	RoleAssistant Role = "assistant"
	RoleFunction  Role = "function"
	RoleTool      Role = "tool"
)

// UnixTime wraps time.Time to handle custom JSON unmarshaling
type UnixTime struct {
	time.Time
}

// UnmarshalJSON parses a Unix timestamp (integer) into a time.Time object
func (ut *UnixTime) UnmarshalJSON(b []byte) error {
	var timestamp int64
	if err := json.Unmarshal(b, &timestamp); err != nil {
		return err
	}

	// Convert seconds since epoch to time.Time
	ut.Time = time.Unix(timestamp, 0)
	return nil
}

// Message is a single message in a conversation.
type Message struct {
	Role       Role       `json:"role"`
	Content    string     `json:"content,omitempty"`
	ToolCalls  []ToolCall `json:"tool_calls,omitempty"`
	ToolCallID string     `json:"tool_call_id,omitempty"`
}

// NewSystemMessage builds a system message.
func NewSystemMessage(content string) Message {
	return Message{Role: RoleSystem, Content: content}
}

// NewUserMessage builds a user message.
func NewUserMessage(content string) Message {
	return Message{Role: RoleUser, Content: content}
}

// NewAssistantMessage builds an assistant message.
func NewAssistantMessage(content string) Message {
	return Message{Role: RoleAssistant, Content: content}
}

// NewToolResultMessage builds a tool-result message.
func NewToolResultMessage(toolCallID, content string) Message {
	return Message{
		Role:       RoleTool,
		Content:    content,
		ToolCallID: toolCallID,
	}
}

// ToolCall is a tool call requested by the assistant.
type ToolCall struct {
	ID       string       `json:"id"`
	Type     string       `json:"type"`
	Function FunctionCall `json:"function"`
}

// FunctionCall is the function name and arguments for a tool call.
type FunctionCall struct {
	Name      string `json:"name"`
	Arguments string `json:"arguments"` // JSON string — parsed later
}

// ToolDefinition is a tool definition sent to the API.
type ToolDefinition struct {
	Type     string             `json:"type"`
	Function FunctionDefinition `json:"function"`
}

// FunctionDefinition is the function metadata within a tool definition.
type FunctionDefinition struct {
	Name        string          `json:"name"`
	Description string          `json:"description"`
	Parameters  json.RawMessage `json:"parameters"` // JSON Schema
}

// ChatCompletionRequest is the request body for chat completions.
type ChatCompletionRequest struct {
	Model         string           `json:"model"`
	Messages      []Message        `json:"messages"`
	Tools         []ToolDefinition `json:"tools,omitempty"`
	Stream        bool             `json:"stream,omitempty"`
	StreamOptions StreamOptions    `json:"stream_options,omitempty"`
}

type StreamOptions struct {
	IncludeUsage bool `json:"include_usage,omitempty"`
}

// ChatCompletionResponse is the non-streaming response.
// TODO: we are streaming only now it's probably safe to remove all the non-streaming structs
type ChatCompletionResponse struct {
	ID      string   `json:"id"`
	Choices []Choice `json:"choices"`
	Usage   *Usage   `json:"usage,omitempty"`
}

type Choice struct {
	Index        int     `json:"index"`
	Message      Message `json:"message"`
	FinishReason string  `json:"finish_reason,omitempty"`
}

type Model struct {
	ID      string   `json:"id"`
	Object  string   `json:"object"`
	Created UnixTime `json:"created"`
	OwnedBy string   `json:"owned_by"`
}

type ModelsResponse struct {
	Data []Model `json:"data"`
}

type StreamChunk struct {
	Choices []struct {
		Index int `json:"index"`
		Delta struct {
			Content          string           `json:"content"`
			ReasoningContent string           `json:"reasoning_content"` // some providers use "reasoning"
			ToolCalls        []StreamToolCall `json:"tool_calls"`
		} `json:"delta"`
		FinishReason string `json:"finish_reason"`
	} `json:"choices"`
	Usage *Usage `json:"usage"`
}

type Usage struct {
	CompletionTokens       uint32                 `json:"completion_tokens"`
	PromptTokens           uint32                 `json:"prompt_tokens"`
	TotalTokens            uint32                 `json:"total_tokens"`
	CompletionTokenDetails CompletionTokenDetails `json:"completion_token_details"`
	PromptTokenDetails     PromptTokenDetails     `json:"prompt_token_details"`
}

type CompletionTokenDetails struct {
	AcceptedPredictionTokens uint32 `json:"accepted_prediction_tokens"`
	AudioTokens              uint32 `json:"audio_tokens"`
	ReasoningTokens          uint32 `json:"reasoning_tokens"`
	RejectedPredictionTokens uint32 `json:"rejected_prediction_tokens"`
}

type PromptTokenDetails struct {
	AudioTokens      uint32 `json:"audio_tokens"`
	CacheWriteTokens uint32 `json:"cache_write_tokens"`
	CachedTokens     uint32 `json:"cached_tokens"`
}

type StreamToolCall struct {
	Index    int    `json:"index"`
	ID       string `json:"id"`
	CallType string `json:"type"`
	Function struct {
		Name      string `json:"name"`
		Arguments string `json:"arguments"`
	} `json:"function"`
}
