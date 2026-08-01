use crate::api::types::Message;

/// Filter messages for API compatibility.
/// Removes tool results for provider tools (like web_search)
/// that the API handlers internally
pub fn filter_messages(messages: &[Message]) -> Vec<Message> {
    let provider_tool_names = ["web_search"];

    let provider_call_ids: Vec<String> = messages
        .iter()
        .filter_map(|m| m.tool_calls.as_ref())
        .flatten()
        .filter(|tc| provider_tool_names.contains(&tc.function.name.as_str()))
        .map(|tc| tc.id.clone())
        .collect();

    messages
        .iter()
        .filter(|m| {
            // Keep all non-tool messages
            if m.role != "tool" {
                return true;
            }
            // Filter out tool results for provider tools
            if let Some(ref id) = m.tool_call_id {
                !provider_call_ids.contains(id)
            } else {
                true
            }
        })
        .cloned()
        .collect()
}
