package tools

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io/fs"
	"maps"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"slices"
	"strings"
	"time"

	"codeberg.org/readeck/go-readability/v2"
	htmltomarkdown "github.com/JohannesKaufmann/html-to-markdown/v2"
	"github.com/tidwall/gjson"

	"github.com/bloveless/mu/api"
	"github.com/bloveless/mu/events"
	"github.com/bloveless/mu/logging"
)

type Registry map[string]*Tool

type Emitter func(ctx context.Context, kind events.Kind, text string)

type Tool struct {
	Definition api.ToolDefinition
	// Exec runs the tool. Progress notes ("reading file: x") are pushed
	// onto sink, the same event channel the agent loop uses, so they are
	// attributed to the calling agent.
	Exec func(ctx context.Context, tc api.ToolCall, emit Emitter) api.Message
}

// NewRegistry creates an empty tool registry.
func NewRegistry() Registry {
	return make(map[string]*Tool)
}

func (tools Registry) Register(name string, t *Tool) error {
	if _, ok := tools[name]; ok {
		return fmt.Errorf("a tool named %s already exists", name)
	}
	tools[name] = t
	return nil
}

func (tools Registry) GetDefinitions() []api.ToolDefinition {
	tds := make([]api.ToolDefinition, 0, len(tools))
	for _, t := range tools {
		tds = append(tds, t.Definition)
	}
	return tds
}

// ExecTool dispatches a tool call to the named tool. Displaying the
// result is the caller's concern; tools only report progress on sink.
func (tools Registry) ExecTool(ctx context.Context, tc api.ToolCall, emit Emitter) api.Message {
	tool, ok := tools[tc.Function.Name]
	if !ok {
		existingTools := slices.Collect(maps.Keys(tools))
		return api.NewToolResultMessage(
			tc.ID,
			fmt.Sprintf(
				"attempted to call a tool that doesn't exist [%s]: existing tools are %v",
				tc.Function.Name,
				existingTools,
			),
		)
	}
	return tool.Exec(ctx, tc, emit)
}

// Read creates a tool that reads and returns the contents of a specified file.
func Read() *Tool {
	return &Tool{
		Definition: api.ToolDefinition{
			Type: "function",
			Function: api.FunctionDefinition{
				Name:        "read",
				Description: "Read and return the contents of a file",
				Parameters: []byte(`{
	                "type": "object",
	                "properties": {
	                    "file_path": {
	                        "type": "string",
	                        "description": "The path to the file to read"
	                    }
	                },
	                "required": ["file_path"]
	            }`),
			},
		},
		Exec: func(ctx context.Context, tc api.ToolCall, emit Emitter) api.Message {
			path := gjson.Get(tc.Function.Arguments, "file_path")
			emit(ctx, events.KindToolProgress, fmt.Sprintf("reading file: %s", path.String()))
			b, err := os.ReadFile(path.String())
			if err != nil {
				return api.NewToolResultMessage(
					tc.ID,
					fmt.Sprintf("tool call for %s failed: %s", tc.Function.Name, err),
				)
			}
			logging.Debug("--- content ---\n%s\n---\n", string(b))
			return api.NewToolResultMessage(tc.ID, string(b))
		},
	}
}

// Edit creates a tool that creates files or replaces one unique, exact string in an existing file.
// An empty old_string creates a file only when the path does not already contain content.
func Edit() *Tool {
	return &Tool{
		Definition: api.ToolDefinition{
			Type: "function",
			Function: api.FunctionDefinition{
				Name: "edit",
				Description: `Edit a file with an exact string replacement.
Provide ` + "`old_string`" + ` (the exact text to find in the
file) and ` + "`new_string`" + ` (the replacement). ` + "`old_string`" + `
must match the file exactly, including whitespace and
indentation, and must be UNIQUE within the file — if it
occurs more than once, include more surrounding context
to make it unique. To CREATE a new file, pass an empty
` + "`old_string`" + ` together with the full file contents as
` + "`new_string`" + `; this is refused if the file already exists.
Always read the current file contents before editing it
so ` + "`old_string`" + ` matches exactly.`,
				Parameters: []byte(`{
	                "type": "object",
	                "required": ["file_path", "old_string", "new_string"],
	                "properties": {
	                    "file_path": {
	                        "type": "string",
	                        "description": "The path of the file to edit"
	                    },
	                    "old_string": {
	                        "type": "string",
	                        "description": "The exact text to find in the file. Must be unique within the file unless it is empty, in which case the file is created with ` + "`new_string`" + ` as its contents (and must not already exist)."
	                    },
	                    "new_string": {
	                        "type": "string",
	                        "description": "The text to replace ` + "`old_string`" + ` with. When ` + "`old_string`" + ` is empty, this is the full contents of the new file."
	                    }
	                }
	            }`),
			},
		},
		Exec: func(ctx context.Context, tc api.ToolCall, emit Emitter) api.Message {
			args := tc.Function.Arguments
			fpResult := gjson.Get(args, "file_path")
			if !fpResult.Exists() {
				return api.NewToolResultMessage(tc.ID, "edit tool: missing required argument: file_path")
			}
			oldResult := gjson.Get(args, "old_string")
			if !oldResult.Exists() {
				return api.NewToolResultMessage(tc.ID, "edit tool: missing required argument: old_string")
			}
			newResult := gjson.Get(args, "new_string")
			if !newResult.Exists() {
				return api.NewToolResultMessage(tc.ID, "edit tool: missing required argument: new_string")
			}
			filePath := fpResult.String()
			if filePath == "" {
				return api.NewToolResultMessage(tc.ID, "edit tool: file_path must not be empty")
			}
			oldStr := oldResult.String()
			newStr := newResult.String()

			// --- Creation mode: old_string is empty ---
			if oldStr == "" {
				existing, err := os.ReadFile(filePath)
				if err == nil && len(existing) > 0 {
					return api.NewToolResultMessage(tc.ID, fmt.Sprintf(
						"edit tool: refused to create file — %s already exists with content; provide a non-empty old_string to perform a replacement instead",
						filePath,
					))
				}
				if !errors.Is(err, fs.ErrNotExist) {
					return api.NewToolResultMessage(tc.ID, fmt.Sprintf(
						"edit tool: unable to read file at path to confirm that writing a new file is safe: %s",
						err,
					))
				}
				if err := os.WriteFile(filePath, []byte(newStr), 0o600); err != nil {
					return api.NewToolResultMessage(tc.ID,
						fmt.Sprintf("edit tool: failed to write file %s: %s", filePath, err))
				}
				emit(ctx, events.KindToolProgress, fmt.Sprintf("file created [%s]", filePath))
				logging.Debug("--- new content ---\n%s\n---\n", newStr)
				return api.NewToolResultMessage(tc.ID, fmt.Sprintf("file created: %s", filePath))
			}

			// --- Replacement mode: old_string is non-empty ---
			content, err := os.ReadFile(filePath)
			if err != nil {
				return api.NewToolResultMessage(tc.ID,
					fmt.Sprintf("edit tool: failed to read file %s: %s", filePath, err))
			}
			contentStr := string(content)
			count := strings.Count(contentStr, oldStr)
			switch {
			case count == 0:
				return api.NewToolResultMessage(tc.ID,
					fmt.Sprintf(
						"edit tool: old_string not found in %s\n\nEnsure old_string matches the file exactly, including whitespace and indentation.\nRead the file first to get the current contents.",
						filePath,
					))
			case count > 1:
				return api.NewToolResultMessage(tc.ID,
					fmt.Sprintf(
						"edit tool: old_string found %d times in %s — it must be unique\n\nInclude more surrounding context in old_string to make it match exactly once.",
						count,
						filePath,
					))
			default:
				// Replace exactly once and write back
				result := strings.Replace(contentStr, oldStr, newStr, 1)
				if err := os.WriteFile(filePath, []byte(result), 0o600); err != nil { //nolint:gosec // allow file path traversal since the agent might need access to other directories
					return api.NewToolResultMessage(tc.ID,
						fmt.Sprintf("edit tool: failed to write file %s: %s", filePath, err))
				}
				emit(ctx, events.KindToolProgress, fmt.Sprintf("edited file [%s]", filePath))
				logging.Debug("--- old string ---\n%s\n--- new string---\n%s\n---\n", oldStr, newStr)
				return api.NewToolResultMessage(tc.ID, fmt.Sprintf("file edited: %s", filePath))
			}
		},
	}
}

// Bash creates a tool that executes a shell command and returns its exit code, standard output, and standard error.
func Bash() *Tool {
	return &Tool{
		Definition: api.ToolDefinition{
			Type: "function",
			Function: api.FunctionDefinition{
				Name:        "bash",
				Description: "Execute a shell command",
				Parameters: []byte(`{
	                "type": "object",
	                "required": ["command"],
	                "properties": {
	                    "command": {
	                    "type": "string",
	                    "description": "The command to execute"
	                    }
	                }
	            }`),
			},
		},
		Exec: func(ctx context.Context, tc api.ToolCall, emit Emitter) api.Message {
			command := gjson.Get(tc.Function.Arguments, "command")
			emit(ctx, events.KindToolProgress, fmt.Sprintf("executing command: %s", command.String()))
			var stdout, stderr bytes.Buffer
			cmd := exec.CommandContext(ctx, "bash", "-c", command.String()) //nolint:gosec // allowing arbitrary command execution is the point
			cmd.Stdout = &stdout
			cmd.Stderr = &stderr
			err := cmd.Run()
			exitCode := 0
			var exitError *exec.ExitError
			switch {
			case err == nil:
				// success
			case errors.As(err, &exitError):
				// The command ran but exited non-zero; report the exit code
				// along with stdout/stderr so the agent can see what happened.
				exitCode = exitError.ExitCode()
			default:
				// Genuine failure to start the shell itself (e.g. bash not found).
				return api.NewToolResultMessage(tc.ID, fmt.Sprintf(
					"command failed to start [%s]: %s", command.String(), err,
				))
			}
			output := fmt.Sprintf(
				"exit code: %d\n--- stdout ---\n%s\n--- stderr ---\n%s",
				exitCode,
				stdout.String(),
				stderr.String(),
			)
			logging.Debug("%s", output)
			return api.NewToolResultMessage(tc.ID, output)
		},
	}
}

// Fetch returns a tool that retrieves a website and converts its readable content to Markdown.
func Fetch() *Tool {
	return &Tool{
		Definition: api.ToolDefinition{
			Type: "function",
			Function: api.FunctionDefinition{
				Name:        "fetch",
				Description: "Fetch the HTML content of a website",
				Parameters: []byte(`{
	                "type": "object",
	                "required": ["url"],
	                "properties": {
	                    "url": {
	                        "type": "string",
	                        "description": "The URL of the website to fetch"
	                    }
	                }
	            }`),
			},
		},
		Exec: func(ctx context.Context, tc api.ToolCall, emit Emitter) api.Message {
			urlArg := gjson.Get(tc.Function.Arguments, "url")
			u, err := url.Parse(urlArg.String())
			if err != nil {
				return api.NewToolResultMessage(tc.ID, fmt.Sprintf("received invalid url [%s]: error: %s", urlArg, err))
			}
			emit(ctx, events.KindToolProgress, fmt.Sprintf("fetching url: %s", u.String()))
			c := http.Client{
				Timeout: 15 * time.Second,
			}
			req, err := http.NewRequestWithContext(ctx, http.MethodGet, u.String(), http.NoBody)
			if err != nil {
				return api.NewToolResultMessage(tc.ID, fmt.Sprintf(
					"unable to create new request for fetching url [%s]: %s", u.String(), err,
				))
			}
			resp, err := c.Do(req)
			if err != nil {
				return api.NewToolResultMessage(tc.ID, fmt.Sprintf(
					"failed to fetch url [%s]; error: %s", u.String(), err),
				)
			}
			defer resp.Body.Close() //nolint:errcheck // the agent doesn't need to know that the close failed
			if resp.StatusCode < 200 || resp.StatusCode > 299 {
				return api.NewToolResultMessage(tc.ID, fmt.Sprintf(
					"status was not successful (2xx) for url [%s]: status code %d", u.String(), resp.StatusCode,
				))
			}
			article, err := readability.FromReader(resp.Body, u)
			if err != nil {
				return api.NewToolResultMessage(tc.ID, fmt.Sprintf(
					"unable to parse response body for url [%s]: error %s", u.String(), err,
				))
			}
			mdBytes, err := htmltomarkdown.ConvertNode(article.Node)
			if err != nil {
				return api.NewToolResultMessage(tc.ID, fmt.Sprintf(
					"unable to convert response to readability article markdown for url [%s]: error %s",
					u.String(),
					err,
				))
			}
			md := string(mdBytes)
			logging.Debug("fetched url markdown:\n--- output ---\n%s\n---\n", md)
			return api.NewToolResultMessage(tc.ID, md)
		},
	}
}
