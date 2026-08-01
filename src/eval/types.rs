use serde::{Deserialize, Serialize};

/// A single evaluation test case.
#[derive(Debug, Clone, Deserialize)]
pub struct EvalCase {
    pub input: String,
    pub expected_tool: String,
    #[serde(default)]
    pub secondary_tools: Vec<String>,
}

/// The result of running one eval case.
#[derive(Debug, Clone, Serialize)]
pub struct EvalResult {
    pub input: String,
    pub expected_tool: String,
    pub actual_tool: Option<String>,
    pub passed: bool,
    pub score: f64,
    pub reason: String,
}

/// Summary of an entire eval suite.
#[derive(Debug, Clone, Serialize)]
pub struct EvalSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub average_score: f64,
    pub results: Vec<EvalResult>,
}

/// A multi-turn evaluation case.
#[derive(Debug, Clone, Deserialize)]
pub struct MultiTurnEvalCase {
    pub input: String,
    pub expected_tools: Vec<String>,
    pub expected_content: String,
}

/// Result of a multi-turn evaluation.
#[derive(Debug, Clone, Serialize)]
pub struct MultiTurnEvalResult {
    pub input: String,
    pub expected_tools: Vec<String>,
    pub actual_tools: Vec<String>,
    pub tool_order_correct: bool,
    pub content_score: f64,
    pub judge_reasoning: String,
    pub passed: bool,
}
