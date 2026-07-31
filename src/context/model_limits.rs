pub struct ModelLimits {
    pub context_window: usize,
    pub max_output: usize,
}

/// Threshold for triggering compaction (percentage of context used).
pub const COMPACTION_THRESHOLD: f64 = 0.75;

pub fn get_model_limits(model: &str) -> ModelLimits {
    match model {
        "mimo-v2.5-free" => ModelLimits {
            context_window: 200_000,
            max_output: 32_000,
        },
        "mimo-v2.5-pro" => ModelLimits {
            context_window: 1_000_000,
            max_output: 131_072,
        },
        _ => ModelLimits {
            context_window: 128_000,
            max_output: 16_384,
        },
    }
}

pub fn should_compact(token_count: usize, model: &str) -> bool {
    let limits = get_model_limits(model);
    let threshold = (limits.context_window as f64 * COMPACTION_THRESHOLD) as usize;
    token_count > threshold
}

pub struct TokenUsageInfo {
    pub used: usize,
    pub limit: usize,
    pub percentage: f64,
    pub threshold: f64,
}

pub fn get_token_usage(token_count: usize, mode: &str) -> TokenUsageInfo {
    let limits = get_model_limits(mode);
    TokenUsageInfo {
        used: token_count,
        limit: limits.context_window,
        percentage: (token_count as f64 / limits.context_window as f64) * 100.0,
        threshold: COMPACTION_THRESHOLD,
    }
}
