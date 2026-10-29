//! The worker answer-block grammar: FILE/EDIT blocks parsed from the model's
//! answer, resolved against approved files, rendered back, and hidden or shown
//! while the answer streams.
use crate::tasks::WorkPolicy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEdit {
    pub path: String,
    pub content: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilePlan {
    pub files: Vec<FileEdit>,
}

pub(super) const FILE_START: &str = "=== FILE: ";
pub(super) const FILE_START_END: &str = " ===";
pub(super) const FILE_END: &str = "=== END FILE ===";
pub(super) const EDIT_START: &str = "=== EDIT: ";
pub(super) const EDIT_END: &str = "=== END EDIT ===";
pub(super) const SEARCH_MARKER: &str = "<<<<<<< SEARCH";
pub(super) const DIVIDER_MARKER: &str = "=======";
pub(super) const REPLACE_MARKER: &str = ">>>>>>> REPLACE";

/// Path of a `=== FILE: path ===` marker line starting at column 0.
fn file_marker(line: &str) -> Option<&str> {
    block_marker(line, FILE_START)
}

/// Path of a `=== EDIT: path ===` marker line starting at column 0.
fn edit_marker(line: &str) -> Option<&str> {
    block_marker(line, EDIT_START)
}

fn block_marker<'a>(line: &'a str, start: &str) -> Option<&'a str> {
    let path = line
        .trim_end()
        .strip_prefix(start)?
        .strip_suffix(FILE_START_END)?
        .trim();
    (!path.is_empty()).then_some(path)
}

fn end_marker(line: &str) -> bool {
    line.trim_end() == FILE_END
}

fn shown_path(path: &str) -> String {
    crate::dashboard::single_line(path)
        .chars()
        .take(200)
        .collect()
}

/// Block content: one wrapping markdown fence layer stripped (only when no other
/// fence line lies inside), one trailing newline.
fn block_content(lines: &[&str]) -> String {
    let mut body = lines;
    let filled = |line: &&str| !line.trim().is_empty();
    if let (Some(first), Some(last)) = (body.iter().position(filled), body.iter().rposition(filled))
    {
        let interior_fence = body[first + 1..last.max(first + 1)]
            .iter()
            .any(|line| line.trim_start().starts_with("```"));
        if first < last
            && !interior_fence
            && body[first].trim_start().starts_with("```")
            && body[last].trim() == "```"
        {
            body = &body[first + 1..last];
        }
    }
    let text = body.join("\n");
    let text = text.trim_end_matches('\n');
    if text.is_empty() {
        String::new()
    } else {
        format!("{text}\n")
    }
}

/// One `=== EDIT: path ===` block: SEARCH/REPLACE pairs applied in order to the
/// approved file's current content (issue #366).
#[derive(Debug)]
pub struct EditBlock {
    pub path: String,
    /// (SEARCH, REPLACE) texts, each line newline-terminated.
    pub pairs: Vec<(String, String)>,
}

struct Parsed {
    files: Vec<FileEdit>,
    edits: Vec<EditBlock>,
}

enum Open<'a> {
    File(&'a str, Vec<&'a str>),
    Edit(&'a str, Vec<&'a str>),
}

/// SEARCH/REPLACE pairs of the lines inside one EDIT block.
fn edit_pairs(path: &str, lines: &[&str]) -> Result<Vec<(String, String)>> {
    #[derive(PartialEq)]
    enum State {
        Between,
        Search,
        Replace,
    }
    let text = |lines: &[&str]| {
        lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>()
    };
    let shown = shown_path(path);
    let (mut pairs, mut state) = (Vec::new(), State::Between);
    let (mut search, mut replace): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    for line in lines {
        let marker = line.trim_end();
        let number = pairs.len() + 1;
        match state {
            State::Between if marker == SEARCH_MARKER => state = State::Search,
            State::Between if marker.is_empty() => {}
            State::Between => {
                return Err(format!(
                    "EDIT block for {shown} has unexpected text outside a {SEARCH_MARKER} pair"
                ))
            }
            State::Search if marker == DIVIDER_MARKER => state = State::Replace,
            State::Search if marker == SEARCH_MARKER || marker == REPLACE_MARKER => {
                return Err(format!(
                    "EDIT block for {shown} pair {number} has a marker line before {DIVIDER_MARKER}"
                ))
            }
            State::Search => search.push(line),
            State::Replace if marker == REPLACE_MARKER => {
                pairs.push((text(&search), text(&replace)));
                search.clear();
                replace.clear();
                state = State::Between;
            }
            State::Replace if marker == SEARCH_MARKER || marker == DIVIDER_MARKER => {
                return Err(format!(
                    "EDIT block for {shown} pair {number} has a marker line before {REPLACE_MARKER}"
                ))
            }
            State::Replace => replace.push(line),
        }
    }
    match state {
        State::Between if pairs.is_empty() => Err(format!(
            "EDIT block for {shown} has no SEARCH/REPLACE pairs"
        )),
        State::Between => Ok(pairs),
        State::Search => Err(format!(
            "EDIT block for {shown} pair {} ends before {DIVIDER_MARKER}",
            pairs.len() + 1
        )),
        State::Replace => Err(format!(
            "EDIT block for {shown} pair {} ends before {REPLACE_MARKER}",
            pairs.len() + 1
        )),
    }
}

/// Parse a worker answer into FILE and EDIT blocks, or a legacy JSON file plan.
/// Content containing a line equal to a marker is not representable and is refused.
fn parse_blocks(answer: &str) -> Result<Parsed> {
    let trimmed = answer.trim();
    if trimmed.starts_with('{') {
        if let Ok(plan) = serde_json::from_str::<FilePlan>(trimmed) {
            return Ok(Parsed {
                files: plan.files,
                edits: Vec::new(),
            });
        }
    }
    let text = answer.replace("\r\n", "\n");
    let mut files = Vec::new();
    let mut edits = Vec::new();
    let mut open: Option<Open> = None;
    for line in text.split('\n') {
        match &mut open {
            None => {
                if let Some(path) = file_marker(line) {
                    open = Some(Open::File(path, Vec::new()));
                } else if let Some(path) = edit_marker(line) {
                    open = Some(Open::Edit(path, Vec::new()));
                }
            }
            Some(Open::File(path, lines)) => {
                if end_marker(line) {
                    files.push(FileEdit {
                        path: path.to_string(),
                        content: block_content(lines),
                    });
                    open = None;
                } else if file_marker(line).is_some() {
                    return Err(format!(
                        "FILE block for {} contains a FILE marker line before {FILE_END}; file content cannot contain marker lines",
                        shown_path(path)
                    ));
                } else {
                    lines.push(line);
                }
            }
            Some(Open::Edit(path, lines)) => {
                if line.trim_end() == EDIT_END {
                    edits.push(EditBlock {
                        path: path.to_string(),
                        pairs: edit_pairs(path, lines)?,
                    });
                    open = None;
                } else if file_marker(line).is_some()
                    || edit_marker(line).is_some()
                    || end_marker(line)
                {
                    return Err(format!(
                        "EDIT block for {} contains a marker line before {EDIT_END}; edit text cannot contain marker lines",
                        shown_path(path)
                    ));
                } else {
                    lines.push(line);
                }
            }
        }
    }
    match open {
        Some(Open::File(path, _)) => {
            return Err(format!(
                "Model output ended inside FILE block for {} (truncated)",
                shown_path(path)
            ))
        }
        Some(Open::Edit(path, _)) => {
            return Err(format!(
                "Model output ended inside EDIT block for {} (truncated)",
                shown_path(path)
            ))
        }
        None => {}
    }
    if files.is_empty() && edits.is_empty() {
        return Err(if trimmed.starts_with('{') {
            "Worker did not return a valid file plan; no files applied".into()
        } else {
            "Model returned no FILE blocks".into()
        });
    }
    Ok(Parsed { files, edits })
}

/// Parse a worker answer: FILE blocks, or a legacy JSON file plan. EDIT blocks need
/// the approved files' content: use [`resolve_answer`] for those.
pub fn parse_answer(answer: &str) -> Result<FilePlan> {
    let parsed = parse_blocks(answer)?;
    if !parsed.edits.is_empty() {
        return Err("EDIT blocks need the approved files' content; resolve them first".into());
    }
    Ok(FilePlan {
        files: parsed.files,
    })
}

/// Occurrences of `needle` in `haystack`, overlapping ones included, counted up to
/// 2: callers only need none, one or many, and a full count is quadratic.
fn occurrences(haystack: &str, needle: &str) -> usize {
    let (mut count, mut from) = (0, 0);
    while count < 2 {
        let Some(at) = haystack[from..].find(needle) else {
            break;
        };
        count += 1;
        from += at
            + haystack[from + at..]
                .chars()
                .next()
                .map_or(1, char::len_utf8);
    }
    count
}

/// Apply `block`'s pairs, in order, to `current`. Each SEARCH must match exactly
/// once in the content so far; any failing pair fails the whole block.
fn apply_edit(block: &EditBlock, current: &str) -> Result<String> {
    let shown = shown_path(&block.path);
    let mut content = current.replace("\r\n", "\n");
    // The prompt shows the file with a newline appended, so a SEARCH may end on the
    // last line; the appended newline is dropped again so none is invented.
    let appended = !content.is_empty() && !content.ends_with('\n');
    if appended {
        content.push('\n');
    }
    for (index, (search, replace)) in block.pairs.iter().enumerate() {
        let number = index + 1;
        if search.is_empty() {
            return Err(format!(
                "EDIT for {shown} pair {number} has an empty SEARCH"
            ));
        }
        let excerpt: String = crate::dashboard::single_line(search.lines().next().unwrap_or(""))
            .chars()
            .take(60)
            .collect();
        match occurrences(&content, search) {
            1 => content = content.replacen(search.as_str(), replace, 1),
            0 => {
                return Err(format!(
                    "EDIT for {shown} pair {number}: SEARCH text not found in the current file (\"{excerpt}\"); copy it exactly from the file, including indentation"
                ))
            }
            _ => {
                return Err(format!(
                    "EDIT for {shown} pair {number}: SEARCH text matches more than once (\"{excerpt}\"); add neighbouring lines so it matches exactly once"
                ))
            }
        }
    }
    if appended && content.ends_with('\n') {
        content.pop();
    }
    Ok(content)
}

/// The answer's file plan, EDIT blocks converted to the equivalent whole-file
/// edits so `validate_plan` and every later path check apply to them unchanged.
/// `current` gives the approved file's baseline text (`None` for a new file); it
/// is asked only for approved paths. Without EDIT blocks this is `parse_answer_for`.
pub fn resolve_answer(
    answer: &str,
    policy: &WorkPolicy,
    current: &dyn Fn(&str) -> Option<String>,
) -> Result<FilePlan> {
    let Ok(Parsed { mut files, edits }) = parse_blocks(answer) else {
        return parse_answer_for(answer, policy);
    };
    if edits.is_empty() {
        return parse_answer_for(answer, policy);
    }
    let mut named = BTreeSet::new();
    for block in &edits {
        let shown = shown_path(&block.path);
        if files.iter().any(|file| file.path == block.path) {
            return Err(format!(
                "Returned {shown} as both a FILE and an EDIT block; use one block per file"
            ));
        }
        if !named.insert(&block.path) {
            return Err(format!(
                "Returned EDIT blocks for {shown} more than once; put all its pairs in one block"
            ));
        }
    }
    for block in &edits {
        let content = if policy.files.contains(&block.path) {
            let base = current(&block.path).ok_or_else(|| {
                format!(
                    "EDIT for {} but the file does not exist yet; create it with a FILE block",
                    shown_path(&block.path)
                )
            })?;
            apply_edit(block, &base)?
        } else {
            // Not approved: never read. validate_plan refuses it as for a FILE block.
            String::new()
        };
        files.push(FileEdit {
            path: block.path.clone(),
            content,
        });
    }
    Ok(FilePlan { files })
}

/// `parse_answer`, plus: with no FILE blocks, one allowed file and exactly one
/// fenced code block, the fence content is that file.
pub fn parse_answer_for(answer: &str, policy: &WorkPolicy) -> Result<FilePlan> {
    let error = match parse_answer(answer) {
        Ok(plan) => return Ok(plan),
        Err(error) => error,
    };
    // Only a fence-only answer: never rescue a FILE-block or JSON attempt that failed.
    let text = answer.replace("\r\n", "\n");
    let attempted = answer.trim().starts_with('{')
        || text
            .split('\n')
            .any(|l| file_marker(l).is_some() || edit_marker(l).is_some());
    if let ([path], Some(content), false) =
        (policy.files.as_slice(), single_fence(answer), attempted)
    {
        return Ok(FilePlan {
            files: vec![FileEdit {
                path: path.clone(),
                content,
            }],
        });
    }
    Err(error)
}

/// The content of the only fenced code block, when exactly one closed fence exists.
fn single_fence(answer: &str) -> Option<String> {
    let text = answer.replace("\r\n", "\n");
    let mut blocks = Vec::new();
    let mut open: Option<Vec<&str>> = None;
    for line in text.split('\n') {
        let fence = line.trim_start().starts_with("```");
        match (&mut open, fence) {
            (None, true) => open = Some(Vec::new()),
            (Some(lines), true) => {
                blocks.push(block_content(lines));
                open = None;
            }
            (Some(lines), false) => lines.push(line),
            (None, false) => {}
        }
    }
    if open.is_some() || blocks.len() != 1 {
        return None;
    }
    blocks.pop().filter(|content| !content.trim().is_empty())
}

/// FILE blocks for `plan`, the inverse of `parse_answer`.
pub fn render_blocks(plan: &FilePlan) -> String {
    plan.files
        .iter()
        .map(|file| {
            let newline = if file.content.is_empty() || file.content.ends_with('\n') {
                ""
            } else {
                "\n"
            };
            format!(
                "{FILE_START}{}{FILE_START_END}\n{}{newline}{FILE_END}\n",
                file.path, file.content
            )
        })
        .collect()
}

/// The previous answer as blocks for a fresh repair: EDIT blocks stay EDIT blocks
/// (they apply to the baseline, which a repair starts from again).
pub(super) fn prior_blocks(answer: &str, policy: Option<&WorkPolicy>) -> Option<String> {
    if let Ok(Parsed { files, edits }) = parse_blocks(answer) {
        if !edits.is_empty() {
            let mut text = render_blocks(&FilePlan { files });
            for edit in &edits {
                text.push_str(&format!("{EDIT_START}{}{FILE_START_END}\n", edit.path));
                for (search, replace) in &edit.pairs {
                    text.push_str(&format!(
                        "{SEARCH_MARKER}\n{search}{DIVIDER_MARKER}\n{replace}{REPLACE_MARKER}\n"
                    ));
                }
                text.push_str(EDIT_END);
                text.push('\n');
            }
            return Some(text);
        }
    }
    match policy {
        Some(policy) => parse_answer_for(answer, policy).ok(),
        None => parse_answer(answer).ok(),
    }
    .map(|plan| render_blocks(&plan))
}

/// A complete markdown fence line: three backticks and an optional info string.
fn fence_line(line: &str) -> bool {
    line.trim()
        .strip_prefix("```")
        .is_some_and(|rest| !rest.contains('`'))
}

/// A streaming line that is, or may still become, a fence line.
fn partial_fence(line: &str) -> bool {
    let line = line.trim_start();
    !line.is_empty() && (line.starts_with("```") || "```".starts_with(line))
}

/// Streamed model text for live display: FILE markers become `▸ path` headings,
/// END markers and markdown fence lines are hidden. Inside a FILE block only the
/// fence wrapping the whole file is hidden; the file's own fences are content.
/// Display only: `parse_answer` reads the raw text.
pub fn display_output(text: &str) -> String {
    let mut shown = String::with_capacity(text.len());
    // Inside a FILE block: content seen yet, a wrapper fence was hidden, and the
    // depth of the file's own fences within it.
    let (mut in_block, mut seen, mut wrapped, mut depth) = (false, false, false, 0usize);
    // Inside a FILE block EDIT marker lines are file content, not markers.
    let mut in_file = false;
    for line in text.split_inclusive('\n') {
        let complete = line.ends_with('\n');
        if let Some(path) = file_marker(line) {
            shown.push_str(&format!("▸ {path}\n"));
            (in_block, in_file, seen, wrapped, depth) = (true, true, false, false, 0);
        } else if let Some(path) = edit_marker(line).filter(|_| !in_file) {
            shown.push_str(&format!("▸ {path} (edit)\n"));
            (in_block, seen, wrapped, depth) = (true, false, false, 0);
        } else if end_marker(line) || (!in_file && line.trim_end() == EDIT_END) {
            (in_block, in_file) = (false, false);
        } else if (!complete && (line.starts_with("===") || partial_fence(line)))
            || (fence_line(line)
                && hides_fence(in_block, &mut seen, &mut wrapped, &mut depth, line))
        {
            // A marker or fence line still streaming stays hidden until complete.
        } else {
            if in_block && !line.trim().is_empty() {
                seen = true;
            }
            shown.push_str(line);
        }
    }
    shown
}

/// Whether the complete fence `line` is hidden, updating the FILE block state.
fn hides_fence(
    in_block: bool,
    seen: &mut bool,
    wrapped: &mut bool,
    depth: &mut usize,
    line: &str,
) -> bool {
    if !in_block {
        return true;
    }
    if !*seen {
        *seen = true;
        *wrapped = true;
        return true;
    }
    if !*wrapped {
        return false;
    }
    if line.trim() != "```" {
        *depth += 1;
        return false;
    }
    if *depth > 0 {
        *depth -= 1;
        return false;
    }
    *wrapped = false;
    true
}
