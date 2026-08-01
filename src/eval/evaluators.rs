use anyhow::Result;
use serde::Deserialize;

use crate::{
    api::{
        client::OpenAIClient,
        types::{ChatCompletionRequest, Message},
    },
    eval::types::EvalSummary,
};

use super::types::{EvalCase, EvalResult};

pub fn evaluate_tool_call(case: &EvalCase, actual_tool: Option<&str>) -> EvalResult {
    let (passed, score, reason) = match actual_tool {
        // Model called a tool
        Some(tool) => {
            if tool == case.expected_tool {
                (true, 1.0, format!("Correct: selected {tool}"))
            } else if case.secondary_tools.contains(&tool.to_string()) {
                (
                    true,
                    0.5,
                    format!("Acceptable: selected {tool} (secondary)"),
                )
            } else if case.expected_tool == "none" {
                (false, 0.0, format!("Expected no tool call, got {tool}"))
            } else {
                (
                    false,
                    0.0,
                    format!("Wrong tool: expected {}, got {tool}", case.expected_tool),
                )
            }
        }
        // Model didn't call any tool
        None => {
            if case.expected_tool == "none" {
                (true, 1.0, "Correct: no tool call".into())
            } else {
                (
                    false,
                    0.0,
                    format!("Expected {}, got no tool call", case.expected_tool),
                )
            }
        }
    };

    EvalResult {
        input: case.input.clone(),
        expected_tool: case.expected_tool.clone(),
        actual_tool: actual_tool.map(String::from),
        passed,
        score,
        reason,
    }
}

/// Summarize a batch of eval results.
pub fn summarize(results: Vec<EvalResult>) -> EvalSummary {
    let total = results.len();
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = total - passed;
    let average_score = if total > 0 {
        results.iter().map(|r| r.score).sum::<f64>() / total as f64
    } else {
        0.0
    };

    super::types::EvalSummary {
        total,
        passed,
        failed,
        average_score,
        results,
    }
}

/// Check if `expected` appears as a subsequence of `actual`.
/// Additional tools is `actual` are allowed.
pub fn is_subsequence(expected: &[String], actual: &[String]) -> bool {
    let mut expected_iter = expected.iter();
    let mut current = expected_iter.next();

    for actual_tool in actual {
        if let Some(expected_tool) = current {
            if actual_tool == expected_tool {
                current = expected_iter.next();
            }
        }
    }

    current.is_none()
}

#[derive(Debug, Deserialize)]
pub struct JudgeResponse {
    score: f64,
    reasoning: String,
}

pub async fn llm_judge(
    client: &OpenAIClient,
    input: &str,
    expected: &str,
    actual: &str,
) -> Result<(f64, String)> {
    let prompt = format!(
        r#"You are an evaluation judge. Score how well the actual response answers the user's question compared to the expected response.

User question: {input}

Expected response should contain: {expected}

Actual response: {actual}

Respond with JSON only:
{{"score": <0.0 to 1.0>, "reasoning": "<brief explanation>"}}"#
    );

    let request = ChatCompletionRequest {
        model: "deepseek-v4-flash-free".into(),
        messages: vec![Message::user(&prompt)],
        tools: None,
        stream: None,
    };

    let response = client.chat_completion(request).await?;

    let content = response
        .choices
        .first()
        .and_then(|c| c.message.content.as_ref())
        .unwrap_or(&String::new())
        .clone();

    // Parse the JSON response
    match serde_json::from_str::<JudgeResponse>(&content) {
        Ok(judge) => Ok((judge.score, judge.reasoning)),
        Err(_) => Ok((0.5, "Could not parse judge response".into())),
    }
}
