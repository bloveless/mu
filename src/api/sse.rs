use serde::Deserialize;

/// A single chunk from the SSE stream.
#[derive(Debug, Deserialize)]
pub struct StreamChunk {
    pub id: Option<String>,
    pub choices: Vec<StreamChoice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
pub struct StreamChoice {
    pub index: usize,
    pub delta: Delta,
    pub finish_reason: Option<String>,
}

/// The incremental content in a stream chunk.
#[derive(Debug, Deserialize)]
pub struct Delta {
    pub role: Option<String>,
    pub content: Option<String>,
    #[serde(alias = "reasoning")]
    pub reasoning_content: Option<String>,
    pub tool_calls: Option<Vec<StreamToolCall>>,
}

/// A tool call fragment from the stream.
#[derive(Debug, Deserialize)]
pub struct StreamToolCall {
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub call_type: Option<String>,
    pub function: Option<StreamFunction>,
}

#[derive(Debug, Deserialize)]
pub struct StreamFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}
