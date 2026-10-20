//! Chat Agent tools (issue #233, PRD #231): the registry of tools a chat turn may
//! call, their Ollama definitions, dispatch, and the capped call -> result -> continue
//! loop ([`run_turn`]).
//!
//! Adding a tool = one function-family file exporting [`Tool`] consts plus one entry
//! in [`TOOLS`]. Every tool has a [`Tier`]; today only [`Tier::Read`] exists and runs
//! without asking. Later slices add mutating tiers whose calls pause the loop for
//! approval; the loop consults the tier before running a call.
//!
//! Tools act on the Coding Workspace (the owner's repository, working tree). Paths are
//! confined to it ([`resolve`]); outputs are capped at [`MAX_OUTPUT`] bytes. Errors are
//! returned to the model as `error: ...` text, never raised.

mod read;
mod run;

pub use run::{run_turn, MAX_STEPS, STEP_CAP_NOTICE};

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
pub static TOOLS: &[Tool] = &[read::READ_FILE, read::LIST_FILES, read::GREP];

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
    match (tool.run)(root, &arguments) {
        Ok(output) => cap(output),
        Err(error) => format!("error: {error}"),
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
    if text.len() <= MAX_OUTPUT {
        return text;
    }
    let mut end = MAX_OUTPUT;
    while !text.is_char_boundary(end) {
        end -= 1;
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
