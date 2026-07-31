use anyhow::Result;
use mu::eval::executors::run_multi_turn;
use std::fs;

use mu::api::client::OpenAIClient;
use mu::eval::evaluators::{is_subsequence, llm_judge};
use mu::eval::mocks::mock_file_registry;
use mu::eval::types::MultiTurnEvalCase;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv()?;

    let api_key = std::env::var("OPENCODE_API_KEY").expect("OPENCODE_API_KEY must be set");
    let client = OpenAIClient::new(api_key);

    let registry = mock_file_registry();
    let definitions = registry.definitions();

    let data = fs::read_to_string("eval_data/agent_multiturn.json")?;
    let cases: Vec<MultiTurnEvalCase> = serde_json::from_str(&data)?;

    println!("Running {} multi-turn eval cases...\n", cases.len());

    let mut total_score = 0.0;
    let mut passed = 0;

    for case in &cases {
        let (actual_tools, response) =
            run_multi_turn(&client, &registry, &definitions, &case.input).await?;

        let order_ok = is_subsequence(&case.expected_tools, &actual_tools);

        let (content_score, reasoning) =
            llm_judge(&client, &case.input, &case.expected_content, &response).await?;

        let overall_passed = order_ok && content_score >= 0.5;

        let status = if overall_passed { "PASS" } else { "FAIL" };
        println!("[{status}] \"{}\"", case.input);
        println!(
            "  Tools: {:?} (order {})",
            actual_tools,
            if order_ok { "OK" } else { "WRONG" },
        );
        println!();

        if overall_passed {
            passed += 1;
        }
        total_score += content_score;
    }

    println!("--- Summary ---");
    println!("Passed: {}/{}", passed, cases.len());
    println!("Avg content score: {:.2}", total_score / cases.len() as f64);

    Ok(())
}
