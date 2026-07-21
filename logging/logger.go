package logging

import (
	"fmt"
	"os"
)

// Define ANSI color codes
const (
	reset  = "\033[0m"
	red    = "\033[31m"
	green  = "\033[32m"
	yellow = "\033[33m"
	blue   = "\033[34m"
	purple = "\033[35m"
	cyan   = "\033[36m"
	white  = "\033[37m"
)

var verbose bool

func Init(v bool) {
	verbose = v
}

func Log(msg string, args ...any) {
	fmt.Printf(msg, args...)
}

func ThinkingLog(msg string, args ...any) {
	fmt.Printf(yellow+msg+reset, args...)
}

func AssistantLog(msg string, args ...any) {
	fmt.Printf(blue+msg+reset, args...)
}

func ToolLog(msg string, args ...any) {
	fmt.Printf(cyan+msg+reset, args...)
}

func Error(msg string, args ...any) {
	fmt.Fprintf(os.Stderr, msg, args...)
}

func Debug(msg string, args ...any) {
	if verbose {
		fmt.Fprintf(os.Stderr, msg, args...)
	}
}
