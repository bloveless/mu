package logging

import (
	"fmt"
	"os"
)

// Define ANSI color codes
const (
	reset     = "\033[0m"
	red       = "\033[31m"
	green     = "\033[32m"
	yellow    = "\033[33m"
	blue      = "\033[34m"
	purple    = "\033[35m"
	cyan      = "\033[36m"
	white     = "\033[37m"
	lightGrey = "\033[90m"
)

var verbose bool

func SetVerbose(v bool) {
	verbose = v
}

// Log prints user facing messages to stdout
func Log(msg string, args ...any) {
	fmt.Printf(blue+msg+reset, args...)
}

// ThinkingLog prints assistant thinking logs to stdout
func ThinkingLog(msg string, args ...any) {
	fmt.Printf(yellow+msg+reset, args...)
}

// AssistantLog prints assistant responses to stdout. Assistant log writes in the default color of the terminal to draw
// attention to it.
func AssistantLog(msg string, args ...any) {
	fmt.Printf(msg, args...)
}

// ToolLog prints tool log formatted messages to stdout
func ToolLog(msg string, args ...any) {
	fmt.Printf(cyan+msg+reset, args...)
}

// ToolResultLog prints tool responses formatted to stdout
func ToolResultLog(msg string, args ...any) {
	fmt.Printf(lightGrey+msg+reset, args...)
}

// Error prints error logs to stderr
func Error(msg string, args ...any) {
	fmt.Fprintf(os.Stderr, msg, args...)
}

// Info prints info logs to stderr
func Info(msg string, args ...any) {
	fmt.Fprintf(os.Stderr, msg, args...)
}

// Debug prints debug logs to stderr
func Debug(msg string, args ...any) {
	if verbose {
		fmt.Fprintf(os.Stderr, msg, args...)
	}
}
