//! The worker answer-block grammar: FILE/EDIT blocks parsed from the model's
//! answer, resolved against approved files, rendered back, and hidden or shown
//! while the answer streams.
use crate::tasks::WorkPolicy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt::Write as _;

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
/// once in the content so far; any failing pair fails the whole block. Matching is
/// done on LF text; a file that uses CRLF throughout is written back with CRLF.
fn apply_edit(block: &EditBlock, current: &str) -> Result<String> {
    let shown = shown_path(&block.path);
    // A file whose every newline is CRLF gets its CRLF back at the end, so an EDIT
    // does not rewrite every line; mixed endings stay LF-normalised.
    let crlf = current.contains("\r\n") && !current.replace("\r\n", "").contains('\n');
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
    if crlf {
        content = content.replace('\n', "\r\n");
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
                let _ = writeln!(text, "{EDIT_START}{}{FILE_START_END}", edit.path);
                for (search, replace) in &edit.pairs {
                    let _ = write!(
                        text,
                        "{SEARCH_MARKER}\n{search}{DIVIDER_MARKER}\n{replace}{REPLACE_MARKER}\n"
                    );
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

/// One display row of streamed model text.
enum Row<'a> {
    /// A FILE or EDIT block starts: its `▸ path` heading.
    Heading(String),
    /// An end marker closed the current block, or a fence closed a snippet.
    End,
    /// A shown line, as streamed (with its newline when complete).
    Text(&'a str, Kind),
}

/// Where a shown line sits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Outside any block or fence: the model's prose.
    Prose,
    /// Content of a FILE or EDIT block.
    Block,
    /// Inside a markdown fence outside any block: a code snippet.
    Snippet,
}

/// Line-by-line state of the display rows: which block or fence the next line is in.
#[derive(Clone, Debug, Default)]
struct Scanner {
    /// Inside a FILE or EDIT block.
    in_block: bool,
    /// Inside a FILE block EDIT marker lines are file content, not markers.
    in_file: bool,
    /// Inside a FILE block: content seen yet, a wrapper fence was hidden, and the
    /// depth of the file's own fences within it.
    seen: bool,
    wrapped: bool,
    depth: usize,
    /// Between markdown fence lines outside any block.
    in_snippet: bool,
}

impl Scanner {
    /// The row `line` shows, if any. A line without its newline is still streaming.
    fn line<'a>(&mut self, line: &'a str) -> Option<Row<'a>> {
        let complete = line.ends_with('\n');
        if let Some(path) = file_marker(line) {
            *self = Self {
                in_block: true,
                in_file: true,
                ..Self::default()
            };
            Some(Row::Heading(format!("▸ {path}")))
        } else if let Some(path) = edit_marker(line).filter(|_| !self.in_file) {
            (self.in_block, self.seen, self.wrapped, self.depth) = (true, false, false, 0);
            self.in_snippet = false;
            Some(Row::Heading(format!("▸ {path} (edit)")))
        } else if end_marker(line) || (!self.in_file && line.trim_end() == EDIT_END) {
            let closed = self.in_block;
            (self.in_block, self.in_file) = (false, false);
            closed.then_some(Row::End)
        } else if !complete && (line.starts_with("===") || partial_fence(line)) {
            // A marker or fence line still streaming stays hidden until complete.
            None
        } else if fence_line(line)
            && hides_fence(
                self.in_block,
                &mut self.seen,
                &mut self.wrapped,
                &mut self.depth,
                line,
            )
        {
            if self.in_block {
                return None;
            }
            let closes = self.in_snippet;
            self.in_snippet = !self.in_snippet;
            closes.then_some(Row::End)
        } else {
            if self.in_block && !line.trim().is_empty() {
                self.seen = true;
            }
            let kind = if self.in_block {
                Kind::Block
            } else if self.in_snippet {
                Kind::Snippet
            } else {
                Kind::Prose
            };
            Some(Row::Text(line, kind))
        }
    }
}

/// The display rows of `text`: FILE markers become headings, END markers and
/// markdown fence lines are hidden. Inside a FILE block only the fence wrapping
/// the whole file is hidden; the file's own fences are content.
fn display_rows(text: &str) -> Vec<Row<'_>> {
    let mut scanner = Scanner::default();
    text.split_inclusive('\n')
        .filter_map(|line| scanner.line(line))
        .collect()
}

/// Rows the live fold keeps; older ones collapse into `… N earlier files`.
const FOLD_ROWS: usize = 64;
/// A streaming line longer than this is counted as it stands and its rest skipped.
const FOLD_LINE: usize = 2048;
/// Characters of one prose line the live fold keeps.
const FOLD_PROSE: usize = 240;

/// One row of the live fold.
#[derive(Clone, Debug)]
enum FoldRow {
    /// A FILE or EDIT block, or a fenced snippet: its heading and line count.
    Block {
        heading: String,
        lines: usize,
        closed: bool,
    },
    /// A line of the model's prose.
    Prose(String),
}

/// The fold of a worker's streamed answer, kept as the text arrives so counts stay
/// exact however much of it the output tail has dropped. Bounded: [`FOLD_ROWS`]
/// rows, a line buffer of [`FOLD_LINE`] bytes. Display only: `parse_answer` reads
/// the raw answer.
#[derive(Clone, Debug, Default)]
pub struct Fold {
    scanner: Scanner,
    /// The streaming line, until its newline arrives (a marker can span chunks).
    pending: String,
    /// Offset of `pending` in the stream.
    pending_start: usize,
    /// Dropping the rest of an over-long line.
    skipping: bool,
    rows: std::collections::VecDeque<FoldRow>,
    /// The last row is a block still taking lines.
    active: bool,
    /// File and snippet rows dropped from the front.
    earlier: usize,
    /// The marker line of the open block and its offset in the stream.
    open: Option<(String, usize)>,
    /// Bytes of the stream seen.
    total: usize,
}

impl Fold {
    /// Whether any text was folded (a progress built without the stream has none).
    pub fn tracked(&self) -> bool {
        self.total > 0
    }

    /// Fold the next chunk of the stream.
    pub fn push(&mut self, chunk: &str) {
        let base = self.total;
        self.total += chunk.len();
        let mut from = 0;
        if self.skipping {
            match chunk.find('\n') {
                Some(at) => (from, self.skipping) = (at + 1, false),
                None => return,
            }
        }
        if self.pending.is_empty() {
            self.pending_start = base + from;
        }
        self.pending.push_str(&chunk[from..]);
        while let Some(at) = self.pending.find('\n') {
            let line: String = self.pending.drain(..=at).collect();
            let start = self.pending_start;
            self.pending_start += line.len();
            self.line(&line, start);
        }
        if self.pending.len() > FOLD_LINE {
            let line = std::mem::take(&mut self.pending);
            self.line(&line, self.pending_start);
            self.skipping = true;
        }
    }

    fn line(&mut self, line: &str, start: usize) {
        match self.scanner.line(line) {
            None => {}
            Some(Row::Heading(heading)) => {
                let marker = match (file_marker(line), edit_marker(line)) {
                    (Some(path), _) => format!("{FILE_START}{path}{FILE_START_END}\n"),
                    (None, Some(path)) => format!("{EDIT_START}{path}{FILE_START_END}\n"),
                    (None, None) => String::new(),
                };
                self.open = Some((marker, start));
                self.rows.push_back(FoldRow::Block {
                    heading,
                    lines: 0,
                    closed: false,
                });
                self.active = true;
            }
            Some(Row::End) => {
                self.open = None;
                if std::mem::take(&mut self.active) {
                    if let Some(FoldRow::Block { closed, .. }) = self.rows.back_mut() {
                        *closed = true;
                    }
                }
            }
            Some(Row::Text(_, Kind::Block | Kind::Snippet)) => {
                match self.rows.back_mut() {
                    Some(FoldRow::Block { lines, .. }) if self.active => *lines += 1,
                    _ => {
                        self.open = None;
                        self.rows.push_back(FoldRow::Block {
                            heading: "▸ code".into(),
                            lines: 1,
                            closed: false,
                        });
                        self.active = true;
                    }
                };
            }
            Some(Row::Text(text, Kind::Prose)) => {
                let kept: String = text.chars().take(FOLD_PROSE).collect();
                let cut = kept.len() < text.len();
                let end = if cut { "…" } else { "" };
                let kept = kept.trim_end_matches('\n');
                self.rows
                    .push_back(FoldRow::Prose(format!("{kept}{end}\n")));
            }
        }
        while self.rows.len() > FOLD_ROWS {
            if let Some(FoldRow::Block { .. }) = self.rows.pop_front() {
                self.earlier += 1;
            }
        }
    }

    /// The fold as [`folded_output`] shows the whole stream, with files dropped for
    /// the row cap as `… N earlier files`.
    pub fn render(&self) -> String {
        let mut fold = self.clone();
        if !fold.pending.is_empty() {
            let line = std::mem::take(&mut fold.pending);
            fold.line(&line, fold.pending_start);
        }
        let mut shown = String::new();
        if fold.earlier > 0 {
            let plural = if fold.earlier == 1 { "" } else { "s" };
            let _ = writeln!(shown, "… {} earlier file{plural}", fold.earlier);
        }
        for row in &fold.rows {
            match row {
                FoldRow::Block {
                    heading,
                    lines,
                    closed,
                } => {
                    let so_far = if *closed { "" } else { " so far" };
                    let count = match lines {
                        1 => "1 line".to_string(),
                        lines => format!("{lines} lines"),
                    };
                    let _ = writeln!(shown, "{heading} · {count}{so_far}");
                }
                FoldRow::Prose(text) => shown.push_str(text),
            }
        }
        shown
    }

    /// `tail`, the last bytes of the stream, ready for [`display_output`]: when the
    /// open block's heading was dropped from it, its marker line goes in front, so the
    /// code shows under the file it belongs to. The tail's first line is dropped then,
    /// as the tail usually starts in the middle of it.
    pub fn expanded(&self, tail: &str) -> String {
        match &self.open {
            Some((marker, start)) if *start < self.total.saturating_sub(tail.len()) => {
                let whole = tail.split_once('\n').map_or("", |(_, rest)| rest);
                format!("{marker}{whole}")
            }
            _ => tail.to_owned(),
        }
    }
}

/// Streamed model text for live display: FILE markers become `▸ path` headings,
/// END markers and markdown fence lines are hidden. Inside a FILE block only the
/// fence wrapping the whole file is hidden; the file's own fences are content.
/// Display only: `parse_answer` reads the raw text.
pub fn display_output(text: &str) -> String {
    let mut shown = String::with_capacity(text.len());
    for row in display_rows(text) {
        match row {
            Row::Heading(heading) => {
                let _ = writeln!(shown, "{heading}");
            }
            Row::End => {}
            Row::Text(line, _) => shown.push_str(line),
        }
    }
    shown
}

/// [`display_output`] with each FILE or EDIT block, and each fenced snippet in the
/// prose, folded to one `▸ path · N lines` row (`N lines so far` while it is still
/// open). The model's prose stays as written. `cut` says the text is a tail whose
/// start was dropped: lines before the first heading then fold into one
/// `… N earlier lines` row, since they may be the middle of a file.
/// Display only: `parse_answer` reads the raw text.
pub fn folded_output(text: &str, cut: bool) -> String {
    fn count(lines: usize) -> String {
        match lines {
            1 => "1 line".into(),
            lines => format!("{lines} lines"),
        }
    }
    fn fold(shown: &mut String, (heading, lines): (String, usize), open: bool) {
        let so_far = if open { " so far" } else { "" };
        let _ = writeln!(shown, "{heading} · {}{so_far}", count(lines));
    }
    let rows = display_rows(text);
    // A tail with no heading at all cannot be told apart from prose: shown as is.
    let mut leading = cut && rows.iter().any(|row| matches!(row, Row::Heading(_)));
    let mut shown = String::new();
    // The block or snippet being folded: its heading and line count.
    let mut current: Option<(String, usize)> = None;
    let mut earlier = 0usize;
    for row in rows {
        match row {
            Row::Heading(heading) => {
                if let Some(open) = current.take() {
                    fold(&mut shown, open, true);
                }
                if std::mem::take(&mut leading) && earlier > 0 {
                    let plural = if earlier == 1 { "" } else { "s" };
                    let _ = writeln!(shown, "… {earlier} earlier line{plural}");
                }
                current = Some((heading, 0));
            }
            Row::End if leading => {}
            Row::End => {
                if let Some(closed) = current.take() {
                    fold(&mut shown, closed, false);
                }
            }
            Row::Text(..) if leading => earlier += 1,
            Row::Text(_, Kind::Block | Kind::Snippet) => match current.as_mut() {
                Some((_, lines)) => *lines += 1,
                None => current = Some(("▸ code".into(), 1)),
            },
            Row::Text(line, Kind::Prose) => shown.push_str(line),
        }
    }
    if let Some(open) = current {
        fold(&mut shown, open, true);
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
