use crate::api::types::Message;

/// Estimate token count for a message.
/// Uses the ~3.75 characters per token heuristic.
pub fn estimate_tokens(text: &str) -> usize {
    (text.len() as f64 / 3.75).ceil() as usize
}

/// Estimate total tokens for a conversation.
pub fn estimate_conversation_tokens(messages: &[Message]) -> usize {
    messages
        .iter()
        .map(|m| {
            let content_tokens = m.content.as_ref().map(|c| estimate_tokens(c)).unwrap_or(0);

            let tool_call_tokens = m
                .tool_calls
                .as_ref()
                .map(|calls| {
                    calls
                        .iter()
                        .map(|tc| {
                            estimate_tokens(&tc.function.name)
                                + estimate_tokens(&tc.function.arguments)
                        })
                        .sum::<usize>()
                })
                .unwrap_or(0);

            // ~4 tokens overhead per message (role, separators)
            content_tokens + tool_call_tokens + 4
        })
        .sum()
}
