//! Tolerant deserialization of transcript JSONL lines.
//!
//! The transcript schema evolves between Claude Code versions, so every
//! field is optional and unknown fields/types are ignored rather than
//! rejected. Only the fields ccq actually reads are declared.

use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, Debug, Default)]
pub struct RawLine {
    #[serde(rename = "type")]
    pub ty: Option<String>,
    pub message: Option<RawMessage>,
    pub timestamp: Option<String>,
    pub cwd: Option<String>,
    #[serde(rename = "isSidechain")]
    pub is_sidechain: Option<bool>,
}

#[derive(Deserialize, Debug, Default)]
pub struct RawMessage {
    pub role: Option<String>,
    pub content: Option<Content>,
    pub usage: Option<Usage>,
}

/// `message.content` appears both as a plain string and as an array of
/// typed blocks. Anything else is captured as `Other` and skipped.
#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<Block>),
    Other(serde::de::IgnoredAny),
}

#[derive(Deserialize, Debug, Default)]
pub struct Block {
    #[serde(rename = "type")]
    pub ty: Option<String>,
    // tool_use
    pub id: Option<String>,
    pub name: Option<String>,
    pub input: Option<Value>,
    // text / thinking
    pub text: Option<String>,
    pub thinking: Option<String>,
    // tool_result
    pub tool_use_id: Option<String>,
    pub is_error: Option<bool>,
    pub content: Option<Value>,
}

#[derive(Deserialize, Debug, Default, Clone, Copy)]
pub struct Usage {
    pub output_tokens: Option<u64>,
}

impl RawLine {
    pub fn blocks(&self) -> &[Block] {
        match self.message.as_ref().and_then(|m| m.content.as_ref()) {
            Some(Content::Blocks(b)) => b,
            _ => &[],
        }
    }

    /// String-form content, when `message.content` is a plain string.
    pub fn string_content(&self) -> Option<&str> {
        match self.message.as_ref().and_then(|m| m.content.as_ref()) {
            Some(Content::Text(s)) => Some(s),
            _ => None,
        }
    }
}

/// Extract readable text from a `tool_result` block's `content`, which is
/// either a plain string or an array of `{type: "text", text}` blocks.
pub fn tool_result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => {
            let mut out = String::new();
            for item in items {
                if let Some(t) = item.get("text").and_then(Value::as_str) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(t);
                }
            }
            out
        }
        _ => String::new(),
    }
}
