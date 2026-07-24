package agent

import (
	"fmt"
	"math"

	"github.com/bloveless/mu/api"
)

// computeCost returns the USD cost for a single response's token usage
// given the provider model's pricing (rates are USD per 1,000,000 tokens).
func computeCost(usage *api.Usage, model *api.ProviderModel) float64 {
	cached := usage.PromptTokenDetails.CachedTokens
	nonCached := safeSubtract(usage.PromptTokens, cached)
	cacheWrite := usage.PromptTokenDetails.CacheWriteTokens
	output := usage.CompletionTokens

	totalCost := float64(nonCached)*model.Cost.Input +
		float64(output)*model.Cost.Output +
		float64(cached)*model.Cost.CacheRead +
		float64(cacheWrite)*model.Cost.CacheWrite
	return totalCost / 1_000_000
}

// FormatUsageLine returns a one-line stats string for a stream response's
// usage and the provider model that produced it. All fields are always
// present — nothing is conditional.
//
// cumulativeCost is the session-level total USD cost (including all
// previous turns and tool calls). The token breakdown and context
// percentage always reflect the current (latest) response.
//
// Returns "" when usage is nil.
func FormatUsageLine(usage *api.Usage, model *api.ProviderModel, cumulativeCost float64) string {
	if usage == nil {
		return ""
	}

	cached := usage.PromptTokenDetails.CachedTokens
	nonCached := safeSubtract(usage.PromptTokens, cached)
	cacheWrite := usage.PromptTokenDetails.CacheWriteTokens
	output := usage.CompletionTokens

	totalPrompt := nonCached + cached + cacheWrite

	// Cache hit rate: what percentage of total prompt tokens were cache hits?
	var cacheHitRate float64
	if totalPrompt > 0 {
		cacheHitRate = float64(cached) / float64(totalPrompt) * 100
	}

	// Context usage: what percentage of the model's context window is used?
	var contextPercent float64
	if model.Limit.Context > 0 {
		contextPercent = float64(usage.TotalTokens) / float64(model.Limit.Context) * 100
	}

	return fmt.Sprintf(
		"↑%s ↓%s R%s W%s CH%.1f%% $%.3f %.1f%%/%s",
		formatTokens(nonCached),
		formatTokens(output),
		formatTokens(cached),
		formatTokens(cacheWrite),
		cacheHitRate,
		cumulativeCost,
		contextPercent,
		formatTokens(uint32(model.Limit.Context)),
	)
}

// safeSubtract returns a - b, clamped to zero to prevent uint32 underflow.
func safeSubtract(a, b uint32) uint32 {
	if b > a {
		return 0
	}
	return a - b
}

// formatTokens formats a token count with engineering-style suffixes:
//
//	< 1,000        → raw integer   e.g. 420
//	< 10,000       → X.Xk          e.g. 1.2k
//	< 1,000,000    → Xk (rounded)  e.g. 200k
//	< 10,000,000   → X.XM          e.g. 1.5M
//	≥ 10,000,000   → XM (rounded)  e.g. 12M
func formatTokens(count uint32) string {
	switch {
	case count < 1000:
		return fmt.Sprintf("%d", count)
	case count < 10000:
		return fmt.Sprintf("%.1fk", float64(count)/1000)
	case count < 1000000:
		return fmt.Sprintf("%dk", int(math.Round(float64(count)/1000)))
	case count < 10000000:
		return fmt.Sprintf("%.1fM", float64(count)/1000000)
	default:
		return fmt.Sprintf("%dM", int(math.Round(float64(count)/1000000)))
	}
}
