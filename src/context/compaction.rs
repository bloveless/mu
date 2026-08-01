use anyhow::Result;

use crate::api::client::OpenAIClient;
use crate::api::types::{ChatCompletionRequest, Message};

const COMPACTION_PROMPT: &str = "You are a conversation summarizer. \
    Summarize the following conversation, preserving all important details, \
    tool results, file contents, and decisions made. Be thorough but concise.";

pub async fn compact_conversation(
    client: &OpenAIClient,
    messages: &[Message],
) -> Result<Vec<Message>> {
    // Keep the system prompt
    let system_msg = messages.first().filter(|m| m.role == "system").cloned();

    // Build a summary of the conversation
    let conversation_text = messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            let content = m.content.as_deref().unwrap_or("");
            let tool_info = m.tool_calls.as_ref().map(|calls| {
                calls
                    .iter()
                    .map(|tc| format!("[tool: {}({})]", tc.function.name, tc.function.arguments))
                    .collect::<Vec<_>>()
                    .join(", ")
            });

            match tool_info {
                Some(info) => format!("{}: {} {}", m.role, content, info),
                None => format!("{}: {}", m.role, content),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let request = ChatCompletionRequest {
        model: "mimo-v2.5-pro".into(),
        messages: vec![
            Message::system(COMPACTION_PROMPT),
            Message::user(&conversation_text),
        ],
        tools: None,
        stream: None,
    };

    let response = client.chat_completion(request).await?;

    let summary = response
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .unwrap_or_else(|| "Conversation summary unavailable".into());

    // Rebuild: system prompt + summary as assistant message
    let mut compacted = Vec::new();

    if let Some(sys) = system_msg {
        compacted.push(sys);
    }

    compacted.push(Message::assistant(&format!(
        "[Previous conversation summary]\n{summary}"
    )));

    Ok(compacted)
}
