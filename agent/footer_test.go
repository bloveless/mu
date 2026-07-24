package agent_test

import (
	"testing"

	"github.com/bloveless/mu/agent"
	"github.com/bloveless/mu/api"
)

func TestFormatUsageLine(t *testing.T) {
	tests := []struct {
		name           string
		usage          *api.Usage
		model          api.ProviderModel
		cumulativeCost float64
		want           string
	}{
		{
			name:           "nil usage returns empty string",
			usage:          nil,
			model:          api.ProviderModel{},
			cumulativeCost: 0,
			want:           "",
		},
		{
			name:           "all zeros with zero-valued model",
			usage:          &api.Usage{},
			model:          api.ProviderModel{},
			cumulativeCost: 0,
			want:           "↑0 ↓0 R0 W0 CH0.0% $0.000 0.0%/0",
		},
		{
			name: "typical usage with cache activity",
			usage: &api.Usage{
				PromptTokens:     1200,
				CompletionTokens: 450,
				TotalTokens:      1650,
				PromptTokenDetails: api.PromptTokenDetails{
					CachedTokens:     200,
					CacheWriteTokens: 100,
				},
			},
			model: api.ProviderModel{
				Cost: api.Cost{
					Input:      3,
					Output:     15,
					CacheRead:  0.3,
					CacheWrite: 3.75,
				},
				Limit: api.Limit{Context: 200000},
			},
			// ↑ = 1200-200 = 1000 ("1.0k")
			// ↓ = 450 ("450")
			// R = 200 ("200")
			// W = 100 ("100")
			// totalPrompt = 1000+200+100 = 1300
			// CH% = 200/1300*100 = 15.4%
			// cost = 3/1e6*1000=0.003 + 15/1e6*450=0.00675 + 0.3/1e6*200=0.00006 + 3.75/1e6*100=0.000375 = 0.010185 → $0.010
			// context% = 1650/200000*100 = 0.8%
			// contextWindow = 200k
			// cumulativeCost = 0.010 (same as per-response since it's a single call)
			cumulativeCost: 0.010,
			want:           "↑1.0k ↓450 R200 W100 CH15.4% $0.010 0.8%/200k",
		},
		{
			name: "high-volume model (claude-style) with large numbers",
			usage: &api.Usage{
				PromptTokens:     350000,
				CompletionTokens: 50000,
				TotalTokens:      400000,
				PromptTokenDetails: api.PromptTokenDetails{
					CachedTokens:     150000,
					CacheWriteTokens: 0,
				},
			},
			model: api.ProviderModel{
				Cost: api.Cost{
					Input:      3,
					Output:     15,
					CacheRead:  0.3,
					CacheWrite: 3.75,
				},
				Limit: api.Limit{Context: 200000},
			},
			// ↑ = 350000-150000 = 200000 ("200k")
			// ↓ = 50000 ("50k")
			// R = 150000 ("150k")
			// W = 0 ("0")
			// totalPrompt = 200000+150000+0 = 350000
			// CH% = 150000/350000*100 = 42.9%
			// cost = 3/1e6*200000=0.6 + 15/1e6*50000=0.75 + 0.3/1e6*150000=0.045 + 3.75/1e6*0=0 = 1.395
			// context% = 400000/200000*100 = 200.0%
			cumulativeCost: 1.395,
			want:           "↑200k ↓50k R150k W0 CH42.9% $1.395 200.0%/200k",
		},
		{
			name: "formatTokens boundary: 999 -> raw",
			usage: &api.Usage{
				PromptTokens: 999,
				TotalTokens:  999,
			},
			model:          api.ProviderModel{Limit: api.Limit{Context: 2000}},
			cumulativeCost: 0,
			want:           "↑999 ↓0 R0 W0 CH0.0% $0.000 50.0%/2.0k",
		},
		{
			name: "formatTokens boundary: 1000 -> 1.0k",
			usage: &api.Usage{
				PromptTokens: 1000,
				TotalTokens:  1000,
				PromptTokenDetails: api.PromptTokenDetails{
					CachedTokens: 500,
				},
			},
			model: api.ProviderModel{
				Cost:  api.Cost{Input: 1},
				Limit: api.Limit{Context: 2000},
			},
			// ↑ = 1000-500 = 500, R = 500
			// totalPrompt = 500+500+0 = 1000, CH% = 500/1000*100 = 50.0%
			// cost = 1/1e6*500 = 0.0005
			cumulativeCost: 0.001,
			want:           "↑500 ↓0 R500 W0 CH50.0% $0.001 50.0%/2.0k",
		},
		{
			name: "formatTokens boundary: 10000 -> 10k (rounded, no decimal)",
			usage: &api.Usage{
				PromptTokens:     10000,
				CompletionTokens: 150,
				TotalTokens:      10000,
			},
			model: api.ProviderModel{
				Cost:  api.Cost{Input: 1, Output: 5},
				Limit: api.Limit{Context: 100000},
			},
			// ↑ = 10000 → "10k", ↓ = 150 → "150"
			// cost = 1/1e6*10000=0.01 + 5/1e6*150=0.00075 = 0.01075 → $0.011
			cumulativeCost: 0.011,
			want:           "↑10k ↓150 R0 W0 CH0.0% $0.011 10.0%/100k",
		},
		{
			name: "formatTokens boundary: 1000000 -> 1.0M",
			usage: &api.Usage{
				PromptTokens: 1000000,
				TotalTokens:  1000000,
			},
			model:          api.ProviderModel{Limit: api.Limit{Context: 2000000}},
			cumulativeCost: 0,
			want:           "↑1.0M ↓0 R0 W0 CH0.0% $0.000 50.0%/2.0M",
		},
		{
			name: "formatTokens boundary: 10000000 -> 10M (rounded, no decimal)",
			usage: &api.Usage{
				PromptTokens: 10000000,
				TotalTokens:  10000000,
			},
			model:          api.ProviderModel{Limit: api.Limit{Context: 20000000}},
			cumulativeCost: 0,
			want:           "↑10M ↓0 R0 W0 CH0.0% $0.000 50.0%/20M",
		},
		{
			name: "cached tokens exceed prompt tokens (defensive)",
			usage: &api.Usage{
				PromptTokens: 100,
				TotalTokens:  100,
				PromptTokenDetails: api.PromptTokenDetails{
					CachedTokens:     200,
					CacheWriteTokens: 50,
				},
			},
			model: api.ProviderModel{
				Cost:  api.Cost{Input: 1, CacheRead: 0.1, CacheWrite: 0.5},
				Limit: api.Limit{Context: 1000},
			},
			// ↑ = 0 (clamped, since 200 > 100)
			// R = 200
			// W = 50
			// totalPrompt = 0+200+50 = 250
			// CH% = 200/250*100 = 80.0%
			// cost = 1/1e6*0=0 + 0.1/1e6*200=0.00002 + 0.5/1e6*50=0.000025 = 0.000045 → $0.000
			cumulativeCost: 0,
			want:           "↑0 ↓0 R200 W50 CH80.0% $0.000 10.0%/1.0k",
		},
		{
			name: "model with reasoning tokens (completion includes them)",
			usage: &api.Usage{
				PromptTokens:     500,
				CompletionTokens: 3000,
				TotalTokens:      3500,
				CompletionTokenDetails: api.CompletionTokenDetails{
					ReasoningTokens: 2000,
				},
			},
			model: api.ProviderModel{
				Cost:  api.Cost{Input: 1, Output: 5},
				Limit: api.Limit{Context: 10000},
			},
			// ↓ = 3000 (includes reasoning, same as pi convention)
			// cost uses total completion = 3000
			// cost = 1/1e6*500=0.0005 + 5/1e6*3000=0.015 = 0.0155 → $0.015 (banker's rounding)
			cumulativeCost: 0.015,
			want:           "↑500 ↓3.0k R0 W0 CH0.0% $0.015 35.0%/10k",
		},
		{
			name: "formatTokens 9999 -> 10.0k",
			usage: &api.Usage{
				PromptTokens: 9999,
				TotalTokens:  9999,
			},
			model:          api.ProviderModel{Limit: api.Limit{Context: 10000}},
			cumulativeCost: 0,
			want:           "↑10.0k ↓0 R0 W0 CH0.0% $0.000 100.0%/10k",
		},
		{
			name: "formatTokens 999999 -> 1000k",
			usage: &api.Usage{
				PromptTokens: 999999,
				TotalTokens:  999999,
			},
			model:          api.ProviderModel{Limit: api.Limit{Context: 1000000}},
			cumulativeCost: 0,
			want:           "↑1000k ↓0 R0 W0 CH0.0% $0.000 100.0%/1.0M",
		},
		{
			name: "formatTokens 9999999 -> 10.0M",
			usage: &api.Usage{
				PromptTokens: 9999999,
				TotalTokens:  9999999,
			},
			model:          api.ProviderModel{Limit: api.Limit{Context: 10000000}},
			cumulativeCost: 0,
			want:           "↑10.0M ↓0 R0 W0 CH0.0% $0.000 100.0%/10M",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := agent.FormatUsageLine(tt.usage, &tt.model, tt.cumulativeCost)
			if got != tt.want {
				t.Errorf("\n got: %q\nwant: %q", got, tt.want)
			}
		})
	}
}
