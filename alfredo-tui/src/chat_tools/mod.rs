//! Chat Agent tools (issue #233, PRD #231): the registry of tools a chat turn may
//! call, their Ollama definitions, dispatch, and the capped call -> result -> continue
//! loop ([`run_turn`]).
//!
//! Adding a tool = one function-family file exporting [`Tool`] consts plus one entry
//! in [`TOOLS`]. Every tool has a [`Tier`]: [`Tier::Read`] runs without asking;
//! [`Tier::Mutating`] calls pause the loop for owner Approval ([`approval`]);
//! [`Tier::Push`] calls always do.
//! [`Tier::Propose`] calls end the turn and prefill the prompt ([`propose`]).
//!
//! Tools act on the Coding Workspace (the owner's repository, working tree). Paths are
//! confined to it ([`resolve`]); outputs are capped at [`MAX_OUTPUT`] bytes. Errors are
//! returned to the model as `error: ...` text, never raised.

pub mod allow;
pub mod approval;
mod budget;
mod edit;
pub(crate) mod git_read;
mod git_write;
pub mod propose;
pub(crate) mod read;
mod run;
mod shell;
pub mod summary;

pub use run::{run_turn, run_turn_limited, MAX_STEPS, STEP_CAP_NOTICE};
pub use shell::stop_all as stop_running_commands;
pub use shell::CANCELLED_RESULT;

use crate::model::ToolCall;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// Largest tool result returned to the model, in bytes (a marker follows a cut).
pub const MAX_OUTPUT: usize = 16 * 1024;

/// How a tool call is governed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Reads the Coding Workspace; runs automatically.
    Read,
    /// Changes the Coding Workspace; each call needs owner Approval.
    Mutating,
    /// Proposes governed work (/plan, /go); changes nothing, so no y/n card: it ends
    /// the turn and prefills the owner's prompt, whose Enter is the confirmation.
    Propose,
    /// Publishes to a remote (issue #237); always asks, never always-allowed.
    Push,
}

impl Tier {
    /// Whether an always-allow decision (#238) may cover this tier. Push never.
    pub fn allowlistable(self) -> bool {
        matches!(self, Tier::Mutating)
    }
}

/// One tool the model may call.
pub struct Tool {
    pub name: &'static str,
    pub tier: Tier,
    pub description: &'static str,
    /// JSON Schema of the arguments object.
    pub parameters: fn() -> Value,
    /// Run against the Coding Workspace root; `Err` becomes `error: ...` for the model.
    pub run: fn(&Path, &Value) -> Result<String, String>,
}

/// The registry, in the order definitions are offered to the model.
pub static TOOLS: &[Tool] = &[
    read::READ_FILE,
    read::LIST_FILES,
    read::GREP,
    git_read::GIT_STATUS,
    git_read::GIT_BRANCH,
    git_read::GIT_LOG,
    git_read::GIT_DIFF,
    edit::WRITE_FILE,
    edit::EDIT_FILE,
    git_write::GIT_COMMIT,
    git_write::GIT_SWITCH,
    git_write::GIT_MERGE,
    git_write::GIT_PUSH,
    shell::RUN_COMMAND,
    propose::PROPOSE_PLAN,
];

/// Ollama `tools` definitions for every registered tool.
pub fn definitions() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|tool| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": (tool.parameters)(),
                }
            })
        })
        .collect()
}

/// Run one call. Unknown tools and bad arguments come back as `error: ...` text.
pub fn dispatch(root: &Path, call: &ToolCall) -> String {
    dispatch_within(root, call, MAX_OUTPUT)
}

/// [`dispatch_within`] under a round [`budget::Budget`]: read tools stop early when it
/// runs out or its cancel flag is set (issue #257).
pub(crate) fn dispatch_budgeted(
    root: &Path,
    call: &ToolCall,
    limit: usize,
    budget: &budget::Budget,
) -> String {
    if let Some(note) = budget.stop_note() {
        return note.into();
    }
    budget::within(budget, || dispatch_within(root, call, limit))
}

/// [`dispatch`] with the output cut to `limit` bytes (at most [`MAX_OUTPUT`]).
pub fn dispatch_within(root: &Path, call: &ToolCall, limit: usize) -> String {
    let Some(tool) = TOOLS.iter().find(|tool| tool.name == call.name) else {
        let names: Vec<_> = TOOLS.iter().map(|tool| tool.name).collect();
        return format!(
            "error: unknown tool {:?}; available: {}",
            call.name.chars().take(64).collect::<String>(),
            names.join(", ")
        );
    };
    let arguments = normalize_arguments(&call.arguments);
    if !arguments.is_object() {
        return "error: arguments must be a JSON object".into();
    }
    let result = if tool.name == "read_file" {
        read::read_file_within(root, &arguments, limit)
    } else {
        (tool.run)(root, &arguments)
    };
    match result {
        Ok(output) => cap_to(output, limit),
        Err(error) => cap_to(format!("error: {error}"), limit),
    }
}

/// Arguments as Ollama expects them: an object. Some models send the object as a JSON
/// string; that is parsed. Null becomes `{}`; anything else is kept for dispatch to refuse.
pub fn normalize_arguments(arguments: &Value) -> Value {
    match arguments {
        Value::String(text) => serde_json::from_str::<Value>(text)
            .ok()
            .filter(Value::is_object)
            .unwrap_or_else(|| Value::Object(Default::default())),
        Value::Null => Value::Object(Default::default()),
        other => other.clone(),
    }
}

/// Cut `text` to [`MAX_OUTPUT`] bytes on a character boundary, with a marker.
pub(crate) fn cap(text: String) -> String {
    cap_to(text, MAX_OUTPUT)
}

/// Cut `text` to `limit` bytes (never more than [`MAX_OUTPUT`]), with the same marker.
pub(crate) fn cap_to(text: String, limit: usize) -> String {
    let limit = limit.min(MAX_OUTPUT);
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    // A budgeted cut of multi-line text ends on a whole line, unless that would discard
    // half the budget (a short header followed by one huge line keeps the plain cut).
    if limit < MAX_OUTPUT && text.as_bytes().get(end) != Some(&b'\n') {
        if let Some(newline) = text[..end]
            .rfind('\n')
            .filter(|at| *at > 0 && *at >= end / 2)
        {
            end = newline;
        }
    }
    format!(
        "{}\n[truncated: {end} of {} bytes]",
        &text[..end],
        text.len()
    )
}

/// Optional string argument.
pub(crate) fn text_arg<'a>(arguments: &'a Value, name: &str) -> Result<Option<&'a str>, String> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(format!("argument {name} must be a string")),
    }
}

/// Never read, listed or searched: the `@path` and planner exclusions (Git metadata,
/// secrets such as `.env*` and `*.key`, generated and vendor trees).
pub(crate) fn excluded(relative: &str) -> bool {
    crate::planning_context::excluded(&relative.to_lowercase())
}

/// Resolve `path` (relative to the workspace) to an existing path inside it.
/// Refuses absolute paths, `..`, excluded paths ([`excluded`]), and symlinks that
/// lead outside.
pub fn resolve(root: &Path, path: &str) -> Result<PathBuf, String> {
    let relative = Path::new(path.trim());
    for component in relative.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => {
                return Err(format!(
                    "path {path:?} must be relative to the Coding Workspace and stay inside it"
                ))
            }
        }
    }
    let refused = || format!("{path} is excluded (Git metadata, secrets or generated files)");
    if excluded(&relative.to_string_lossy()) {
        return Err(refused());
    }
    let root = root
        .canonicalize()
        .map_err(|_| "Coding Workspace is not available".to_string())?;
    let full = root
        .join(relative)
        .canonicalize()
        .map_err(|_| format!("no such file or directory: {path}"))?;
    let inside = full.strip_prefix(&root).map_err(|_| {
        format!("path {path:?} leaves the Coding Workspace through a symlink; refused")
    })?;
    if excluded(&inside.to_string_lossy().replace('\\', "/")) {
        return Err(refused());
    }
    Ok(full)
}

/// Workspace-relative display form with `/` separators.
pub(crate) fn display(root: &Path, path: &Path) -> String {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    path.strip_prefix(&root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod cap_tests {
    use super::cap_to;

    #[test]
    fn budgeted_cut_keeps_the_body_of_a_header_plus_one_long_line() {
        let text = format!("exit code 0 \u{b7} 1 line\n{}", "x".repeat(20_000));
        let total = text.len();
        let out = cap_to(text, 4000);
        let (kept, footer) = out.rsplit_once('\n').unwrap();
        assert!(kept.len() > 2000, "kept only {} bytes", kept.len());
        assert_eq!(
            footer,
            format!("[truncated: {} of {total} bytes]", kept.len())
        );
    }

    #[test]
    fn budgeted_cut_of_multi_line_text_still_ends_on_a_whole_line() {
        let text: String = (0..400).map(|n| format!("line {n:05} filler\n")).collect();
        let out = cap_to(text, 4000);
        let (kept, footer) = out.rsplit_once('\n').unwrap();
        assert!(kept.ends_with("filler"), "{kept}");
        assert!(kept.len() >= 2000 && kept.len() <= 4000);
        assert!(footer.starts_with("[truncated: "), "{footer}");
    }

    #[test]
    fn budgeted_cut_backs_up_only_from_the_half_budget_on() {
        // limit 10: a newline at index 5 (end / 2) backs up; at index 4 the plain cut stays.
        let at_half = cap_to("aaaaa\nbbbbbbbbbbbbbb".into(), 10);
        assert!(at_half.starts_with("aaaaa\n[truncated: 5 of "), "{at_half}");
        let before_half = cap_to("aaaa\nbbbbbbbbbbbbbbb".into(), 10);
        assert!(
            before_half.starts_with("aaaa\nbbbbb\n[truncated: 10 of "),
            "{before_half}"
        );
        // A leading newline never backs up to an empty body.
        let leading = cap_to("\nabcdefgh".into(), 1);
        assert!(leading.starts_with("\n\n[truncated: 1 of "), "{leading}");
    }
}
