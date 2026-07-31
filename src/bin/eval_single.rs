use anyhow::Result;
use std::fs;

use mu::agent::tool_registry::ToolRegistry;
use mu::api::client::OpenAIClient;
use mu::eval::evaluators::{evaluate_tool_call, summarize};
use mu::eval::executors::run_single_turn;
use mu::eval::types::EvalCase;
use mu::tools::file::{ListFilesTool, ReadFileTool};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv()?;

    let api_key = std::env::var("OPENCODE_API_KEY").expect("OPENCODE_API_KEY must be set");

    let client = OpenAIClient::new(api_key);

    // Build registry
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadFileTool));
    registry.register(Box::new(ListFilesTool));

    let definitions = registry.definitions();

    // Load test data
    let data = fs::read_to_string("eval_data/file_tools.json")?;
    let cases: Vec<EvalCase> = serde_json::from_str(&data)?;

    println!("Running {} eval cases...\n", cases.len());

    let mut results = Vec::new();

    for case in &cases {
        let actual = run_single_turn(&client, &definitions, &case.input).await?;
        let result = evaluate_tool_call(case, actual.as_deref());

        let status = if result.passed { "PASS" } else { "FAIL" };
        println!("[{status}] \"{}\" → {}", result.input, result.reason);

        results.push(result)
    }

    let summary = summarize(results);

    println!("\n--- Summary ---");
    println!(
        "Passed: {}/{} ({:.0}%)",
        summary.passed,
        summary.total,
        summary.average_score * 100.0
    );
    if summary.failed > 0 {
        println!("Failed: {}", summary.failed);
    }

    Ok(())
}
