//! A tool call the model wrote as reply text (issue #660). Only recognised, never run:
//! nothing derived from model text reaches the tool path.

use serde_json::Value;

/// What the owner sees when a reply is only a tool call written as text.
pub const NOTICE: &str = "The model wrote a tool call as text; nothing ran. Ctrl+R retries.";

const OPEN: &str = "<tool_call>";
const CLOSE: &str = "</tool_call>";

/// Whether the whole trimmed `reply` is one JSON object `{"name", "arguments"}` naming a
/// registered tool, or only `<tool_call>` blocks that each do. Prose around the call, code
/// fences and unknown tool names do not match.
#[must_use]
pub fn is_text_tool_call(reply: &str) -> bool {
    let text = reply.trim();
    if text.starts_with(OPEN) {
        let mut rest = text;
        while let Some(after) = rest.strip_prefix(OPEN) {
            let Some((inner, tail)) = after.split_once(CLOSE) else {
                return false;
            };
            if !names_registered_tool(inner) {
                return false;
            }
            rest = tail.trim_start();
        }
        return rest.is_empty();
    }
    names_registered_tool(text)
}

fn names_registered_tool(text: &str) -> bool {
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(text.trim()) else {
        return false;
    };
    object.get("arguments").is_some_and(Value::is_object)
        && object
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| super::TOOLS.iter().any(|tool| tool.name == name))
}
