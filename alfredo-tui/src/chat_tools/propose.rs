//! Propose tier (issue #239, ADR 0004 "When chat hands off"): `propose_plan` lets the
//! Chat Agent hand large or risky work to `/plan` or `/go` instead of doing it.
//!
//! The call starts nothing and needs no y/n card: it ends the turn and the session
//! prefills the owner's prompt with `/plan <goal>` or `/go <goal>` ([`prefill`]).
//! The editable prompt is the confirmation: Enter starts it through the ordinary
//! command path; clearing the prompt drops it. Text the owner already typed is kept.
use super::{text_arg, Tier, Tool};
use crate::model::ToolStep;
use serde_json::{json, Value};
use std::path::Path;

/// Longest goal put in the prompt, in bytes.
pub const MAX_GOAL: usize = 2000;
/// Result prefix; the command follows on the same line.
const PREFIX: &str = "proposed: ";

pub const PROPOSE_PLAN: Tool = Tool {
    name: "propose_plan",
    tier: Tier::Propose,
    description: "Put /plan or /go <goal> in the owner's prompt; ends your turn.",
    parameters: || {
        json!({
            "type": "object",
            "properties": {
                "goal": {"type": "string"},
                "mode": {"type": "string", "enum": ["plan", "go"]},
                "reason": {"type": "string"}
            },
            "required": ["goal", "mode", "reason"]
        })
    },
    run: propose_plan,
};

fn propose_plan(_root: &Path, arguments: &Value) -> Result<String, String> {
    let mode = match text_arg(arguments, "mode")? {
        Some("plan") => "plan",
        Some("go") => "go",
        _ => return Err("mode must be \"plan\" or \"go\"".into()),
    };
    let goal = clean(text_arg(arguments, "goal")?.unwrap_or_default());
    if goal.is_empty() {
        return Err("goal is required".into());
    }
    let reason = clean(text_arg(arguments, "reason")?.unwrap_or_default());
    Ok(format!(
        "{PREFIX}/{mode} {goal}\nreason: {reason}\nThe owner's prompt now holds this command; nothing starts until they press Enter. Your turn ends here."
    ))
}

/// One line: control characters dropped, whitespace collapsed, cut to [`MAX_GOAL`].
fn clean(text: &str) -> String {
    let mut line = text
        .split_whitespace()
        .map(|word| word.chars().filter(|c| !hidden(*c)).collect::<String>())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if line.len() > MAX_GOAL {
        let mut end = MAX_GOAL;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        line.truncate(end);
    }
    line
}

/// Controls plus format (Cf) characters: bidi overrides and isolates, zero-width and
/// invisible marks could make the prompt show other text than Enter submits.
pub(crate) fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061C}'
                | '\u{06DD}'
                | '\u{070F}'
                | '\u{08E2}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{110BD}'
                | '\u{110CD}'
                | '\u{13430}'..='\u{1343F}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0001}'
                | '\u{E0020}'..='\u{E007F}'
        )
}

/// True when `call` is a `propose_plan` call that dispatch would accept.
pub(crate) fn valid(call: &crate::model::ToolCall) -> bool {
    call.name == PROPOSE_PLAN.name && propose_plan(Path::new(""), &call.arguments).is_ok()
}

/// The command a step proposes: the first successful `propose_plan` result.
pub fn prefill(step: &ToolStep) -> Option<String> {
    step.calls
        .iter()
        .zip(&step.results)
        .filter(|(call, _)| call.name == PROPOSE_PLAN.name)
        .find_map(|(_, result)| {
            let command = result.strip_prefix(PREFIX)?.lines().next()?;
            (command.starts_with("/plan ") || command.starts_with("/go "))
                .then(|| command.to_string())
        })
}
