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
    description: "Hand large or risky work to /plan or /go: puts the command in the owner's prompt and ends your turn; the owner starts it.",
    parameters: || {
        json!({
            "type": "object",
            "properties": {
                "goal": {"type": "string", "description": "One-sentence goal"},
                "mode": {"type": "string", "enum": ["plan", "go"], "description": "plan = draft for review; go = plan and run"},
                "reason": {"type": "string", "description": "Why not in chat"}
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
        .map(|word| word.chars().filter(|c| !c.is_control()).collect::<String>())
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
