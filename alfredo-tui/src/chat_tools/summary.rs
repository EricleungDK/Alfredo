//! One-line summaries of tool calls for the transcript `⚙` row (issue #274): the path
//! or primary argument first, long text cut by characters. Display only; nothing here
//! decides what runs.

use serde_json::Value;

/// Tools that deliberately use the generic "all values" row. Empty today; a new tool
/// must either get a summary in [`summarize`] or be listed here.
pub const GENERIC: &[&str] = &[];

/// Longest quoted text or path shown in a row, in characters.
const MAX_TEXT: usize = 60;

/// The argument part of the row for a registered tool, or `None` when the tool has no
/// summary (the caller then falls back to listing the values). `arguments` must be the
/// normalised object.
pub fn summarize(name: &str, arguments: &Value) -> Option<String> {
    let text = |key: &str| arguments.get(key).and_then(Value::as_str);
    let flag = |key: &str| arguments.get(key).and_then(Value::as_bool) == Some(true);
    let path = |key: &str| text(key).map_or_else(|| "?".to_string(), |value| cut(value, false));
    let quoted = |key: &str| text(key).map_or_else(|| "?".to_string(), |value| cut(value, true));
    Some(match name {
        "read_file" => match arguments.get("start_line").and_then(Value::as_u64) {
            Some(line) if line > 1 => format!("{} (from line {line})", path("path")),
            _ => path("path"),
        },
        "list_files" => text("path").map_or_else(|| ".".into(), |value| cut(value, false)),
        "grep" => {
            let place = text("path").map_or_else(|| ".".into(), |value| cut(value, false));
            let case = if flag("ignore_case") {
                " (ignore case)"
            } else {
                ""
            };
            format!("{} in {place}{case}", quoted("pattern"))
        }
        "git_status" | "git_branch" => String::new(),
        "git_log" => {
            let reference = text("ref").map_or_else(|| "HEAD".into(), |value| cut(value, false));
            match arguments.get("max_count").and_then(Value::as_u64) {
                Some(count) => format!("{reference} ({count} commits)"),
                None => reference,
            }
        }
        "git_diff" => {
            let range = match (text("base"), text("target")) {
                (Some(base), Some(target)) => {
                    format!("{}...{}", cut(base, false), cut(target, false))
                }
                (Some(base), None) => cut(base, false),
                _ if flag("staged") => "staged".into(),
                _ => "working tree".into(),
            };
            match text("path") {
                Some(only) => format!("{range} {}", cut(only, false)),
                None => range,
            }
        }
        "write_file" => {
            let lines = text("content").map_or(0, |content| content.lines().count());
            let unit = if lines == 1 { "line" } else { "lines" };
            format!("{} ({lines} {unit})", path("path"))
        }
        "edit_file" => path("path"),
        "git_commit" => {
            let scope = match arguments.get("files").and_then(Value::as_array) {
                Some(files) if !files.is_empty() => {
                    let unit = if files.len() == 1 { "file" } else { "files" };
                    format!("{} {unit}", files.len())
                }
                _ => "staged".into(),
            };
            format!("{} ({scope})", quoted("message"))
        }
        "git_switch" => {
            let branch = path("branch");
            if flag("create") {
                format!("{branch} (new)")
            } else {
                branch
            }
        }
        "git_merge" => match text("target") {
            Some(target) => format!("{} into {}", path("source"), cut(target, false)),
            None => path("source"),
        },
        "git_push" => {
            let remote = text("remote").map_or_else(|| "origin".into(), |value| cut(value, false));
            match text("branch") {
                Some(branch) => format!("{remote}/{}", cut(branch, false)),
                None => format!("{remote} (current branch)"),
            }
        }
        "run_command" => {
            let mut notes = Vec::new();
            if let Some(cwd) = text("cwd") {
                notes.push(format!("in {}", cut(cwd, false)));
            }
            if flag("escalate") {
                notes.push("on host".to_string());
            }
            let command = quoted_raw(text("command"));
            if notes.is_empty() {
                command
            } else {
                format!("{command} ({})", notes.join(", "))
            }
        }
        "propose_plan" => {
            let mode = match text("mode") {
                Some("plan") => "/plan",
                Some("go") => "/go",
                _ => "/?",
            };
            format!("{mode} {}", quoted("goal"))
        }
        _ => return None,
    })
}

/// A shell command shown as written (not quoted), cut like other text.
fn quoted_raw(command: Option<&str>) -> String {
    command.map_or_else(|| "?".to_string(), |value| cut(value, false))
}

/// `text` cut to [`MAX_TEXT`] characters (never bytes) and its first line, with `…`
/// when anything was dropped; `quote` wraps it in double quotes.
fn cut(text: &str, quote: bool) -> String {
    let first = text.lines().next().unwrap_or("");
    let mut shown: String = first.chars().take(MAX_TEXT).collect();
    if shown.chars().count() < first.chars().count() || text.lines().nth(1).is_some() {
        shown.push('…');
    }
    if quote {
        format!("\"{shown}\"")
    } else {
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_tools::TOOLS;
    use serde_json::json;

    fn row(name: &str, arguments: Value) -> String {
        summarize(name, &arguments).unwrap_or_else(|| panic!("{name} has no summary"))
    }

    #[test]
    fn every_registered_tool_has_a_summary_or_is_listed_generic() {
        for tool in TOOLS {
            let summarised = summarize(tool.name, &json!({})).is_some();
            let generic = GENERIC.contains(&tool.name);
            assert!(
                summarised != generic,
                "{} must have a summary or be listed in GENERIC (not both)",
                tool.name
            );
        }
    }

    #[test]
    fn unknown_tools_have_no_summary() {
        assert_eq!(summarize("frobnicate", &json!({"a": 1})), None);
    }

    #[test]
    fn issue_examples() {
        let edit = json!({"new_text": "a", "old_text": "A1", "path": "README.md"});
        assert_eq!(row("edit_file", edit), "README.md");
        let content = "x\n".repeat(42);
        assert_eq!(
            row(
                "write_file",
                json!({"path": "src/x.rs", "content": content})
            ),
            "src/x.rs (42 lines)"
        );
        assert_eq!(
            row(
                "git_commit",
                json!({"message": "add new file", "files": ["new.txt"]})
            ),
            "\"add new file\" (1 file)"
        );
        assert_eq!(
            row("grep", json!({"pattern": "TODO", "path": "src/"})),
            "\"TODO\" in src/"
        );
    }

    #[test]
    fn read_tools() {
        assert_eq!(row("read_file", json!({"path": "a.rs"})), "a.rs");
        assert_eq!(
            row("read_file", json!({"path": "a.rs", "start_line": 40})),
            "a.rs (from line 40)"
        );
        assert_eq!(row("list_files", json!({})), ".");
        assert_eq!(row("list_files", json!({"path": "src"})), "src");
        assert_eq!(row("grep", json!({"pattern": "x"})), "\"x\" in .");
        assert_eq!(
            row("grep", json!({"pattern": "x", "ignore_case": true})),
            "\"x\" in . (ignore case)"
        );
        assert_eq!(row("git_status", json!({})), "");
        assert_eq!(row("git_branch", json!({})), "");
        assert_eq!(row("git_log", json!({})), "HEAD");
        assert_eq!(
            row("git_log", json!({"ref": "main", "max_count": 5})),
            "main (5 commits)"
        );
        assert_eq!(row("git_diff", json!({})), "working tree");
        assert_eq!(row("git_diff", json!({"staged": true})), "staged");
        assert_eq!(
            row(
                "git_diff",
                json!({"base": "main", "target": "alfredo/go-1", "path": "src"})
            ),
            "main...alfredo/go-1 src"
        );
        assert_eq!(row("git_diff", json!({"base": "main"})), "main");
    }

    #[test]
    fn write_tools() {
        assert_eq!(
            row("write_file", json!({"path": "a", "content": "one"})),
            "a (1 line)"
        );
        assert_eq!(
            row("write_file", json!({"path": "a", "content": ""})),
            "a (0 lines)"
        );
        assert_eq!(row("git_commit", json!({"message": "m"})), "\"m\" (staged)");
        assert_eq!(
            row("git_commit", json!({"message": "m", "files": ["a", "b"]})),
            "\"m\" (2 files)"
        );
        assert_eq!(row("git_switch", json!({"branch": "x"})), "x");
        assert_eq!(
            row("git_switch", json!({"branch": "x", "create": true})),
            "x (new)"
        );
        assert_eq!(row("git_merge", json!({"source": "x"})), "x");
        assert_eq!(
            row("git_merge", json!({"source": "x", "target": "main"})),
            "x into main"
        );
        assert_eq!(row("git_push", json!({})), "origin (current branch)");
        assert_eq!(
            row("git_push", json!({"remote": "up", "branch": "x"})),
            "up/x"
        );
        assert_eq!(row("run_command", json!({"command": "ls"})), "ls");
        assert_eq!(
            row(
                "run_command",
                json!({"command": "ls", "cwd": "src", "escalate": true})
            ),
            "ls (in src, on host)"
        );
        assert_eq!(
            row(
                "propose_plan",
                json!({"goal": "fix it", "mode": "go", "reason": "r"})
            ),
            "/go \"fix it\""
        );
    }

    #[test]
    fn long_text_is_cut_by_characters_on_one_line() {
        let long = "é".repeat(200);
        let shown = row("git_commit", json!({"message": long}));
        assert!(shown.starts_with('"') && shown.contains('…'));
        assert!(shown.chars().count() < 100, "{shown}");
        let multi = row("grep", json!({"pattern": "a\nb\nc"}));
        assert!(!multi.contains('\n'), "{multi:?}");
        let path = row("edit_file", json!({"path": "d/".repeat(100)}));
        assert!(path.chars().count() <= MAX_TEXT + 1, "{path}");
    }

    #[test]
    fn missing_or_mistyped_arguments_do_not_panic() {
        for tool in TOOLS {
            for arguments in [json!({}), json!({"path": 5, "message": [1], "files": "x"})] {
                let _ = summarize(tool.name, &arguments);
            }
        }
        assert_eq!(row("edit_file", json!({})), "?");
    }
}
