//! Per-session cache of rendered chat message bodies and their wrapped heights.
//!
//! `ui::draw` takes `&App`, so the cache sits behind a `RefCell`. An entry is valid only
//! while the message text is byte-identical, so a stale entry cannot be shown. Appending
//! a streamed token changes only the last message and rebuilds only that entry; a resize
//! recomputes heights from the cached lines.
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use std::cell::RefCell;
use unicode_width::UnicodeWidthChar;

fn code_style() -> Style {
    Style::default().fg(Color::Cyan)
}

fn fence_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn bold_style() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

/// Bidi controls would reorder what the transcript shows (issue #400); `safe` keeps them,
/// so model text shows them as `<U+XXXX>`, as the approval card does.
fn show_bidi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}')
        {
            out.push_str(&format!("<U+{:04X}>", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// A line with an opening or closing code fence marker.
fn is_fence(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

/// Left-to-right inline scan: `**bold**` and `` `code` `` lose their markers and get a
/// style; a marker with no closing partner stays as typed.
fn inline(text: &str, base: Style, spans: &mut Vec<Span<'static>>) {
    let mut plain = String::new();
    let mut rest = text;
    let flush = |plain: &mut String, spans: &mut Vec<Span<'static>>| {
        if !plain.is_empty() {
            spans.push(Span::styled(std::mem::take(plain), base));
        }
    };
    while let Some(c) = rest.chars().next() {
        let marker = if rest.starts_with("**") {
            Some(("**", base.patch(bold_style())))
        } else if c == '`' {
            Some(("`", base.patch(code_style())))
        } else {
            None
        };
        if let Some((mark, style)) = marker {
            let body = &rest[mark.len()..];
            if let Some(end) = body.find(mark).filter(|end| *end > 0) {
                flush(&mut plain, spans);
                spans.push(Span::styled(body[..end].to_owned(), style));
                rest = &body[end + mark.len()..];
                continue;
            }
        }
        plain.push(c);
        rest = &rest[c.len_utf8()..];
    }
    flush(&mut plain, spans);
}

/// Bullet marker (`-`, `*` or `1.`) at the start of `line`: its byte length including the
/// indent and the space after it.
fn bullet_len(line: &str) -> Option<usize> {
    let body = line.trim_start();
    let indent = line.len() - body.len();
    let digits = body.bytes().take_while(u8::is_ascii_digit).count();
    let marker = if body.starts_with("- ") || body.starts_with("* ") {
        1
    } else if (1..=9).contains(&digits) && body[digits..].starts_with(". ") {
        digits + 1
    } else {
        return None;
    };
    Some(indent + marker + 1)
}

fn heading(line: &str) -> bool {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    (1..=6).contains(&hashes) && line[hashes..].starts_with(' ')
}

/// Styled body lines of an assistant message, each flagged when it is a code or fence row
/// (cut at the pane width rather than wrapped). `text` is already sanitised.
fn markdown(text: &str) -> (Vec<Line<'static>>, Vec<bool>) {
    let mut lines = Vec::new();
    let mut cut = Vec::new();
    let mut in_fence = false;
    for line in text.lines() {
        if is_fence(line) {
            in_fence = !in_fence;
            lines.push(Line::styled(line.to_owned(), fence_style()));
            cut.push(true);
        } else if in_fence {
            lines.push(Line::styled(line.to_owned(), code_style()));
            cut.push(true);
        } else if heading(line) {
            let style = bold_style().fg(Color::Cyan);
            lines.push(Line::styled(line.to_owned(), style));
            cut.push(false);
        } else {
            let mut spans = Vec::new();
            let mut rest = line;
            if let Some(len) = bullet_len(line) {
                spans.push(Span::styled(line[..len].to_owned(), code_style()));
                rest = &line[len..];
            }
            inline(rest, Style::default(), &mut spans);
            lines.push(Line::from(spans));
            cut.push(false);
        }
    }
    (lines, cut)
}

/// `line` limited to `width` columns (code rows are cut, not wrapped).
pub fn cut_line(line: &Line<'static>, width: u16) -> Line<'static> {
    let mut left = usize::from(width);
    let mut out = Line::default();
    out.style = line.style;
    for span in &line.spans {
        let mut kept = String::new();
        for c in span.content.chars() {
            let w = c.width().unwrap_or(0);
            if w > left {
                left = 0;
                break;
            }
            left -= w;
            kept.push(c);
        }
        if !kept.is_empty() {
            out.spans.push(Span::styled(kept, span.style));
        }
        if left == 0 {
            break;
        }
    }
    out
}

/// Rows one logical line occupies at `width` columns when wrapped.
pub fn line_height(line: &Line<'_>, width: u16) -> usize {
    if line.width() <= usize::from(width) {
        1
    } else {
        Paragraph::new(line.clone())
            .wrap(Wrap { trim: false })
            .line_count(width)
            .max(1)
    }
}

pub struct Entry {
    /// The message text these lines were built from.
    content: String,
    /// Body lines (without the heading and the trailing blank line).
    pub lines: Vec<Line<'static>>,
    /// Per line: a code or fence row, one row tall and cut at the pane width.
    pub cut: Vec<bool>,
    width: Option<u16>,
    pub heights: Vec<usize>,
}

#[derive(Default)]
pub struct Inner {
    entries: Vec<Entry>,
    builds: usize,
    height_passes: usize,
}

impl Inner {
    /// Make entry `index..` match `messages` (text, styled as Markdown), rebuilding only
    /// entries whose text changed.
    pub fn sync<'a>(&mut self, messages: impl ExactSizeIterator<Item = (&'a str, bool)>) {
        let count = messages.len();
        for (index, (text, styled)) in messages.enumerate() {
            if self.entries.get(index).is_some_and(|e| e.content == text) {
                continue;
            }
            self.builds += 1;
            let shown = crate::dashboard::expand_tabs(&crate::dashboard::safe(text));
            let (lines, cut) = if styled {
                markdown(&show_bidi(&shown))
            } else {
                let lines: Vec<Line<'static>> = shown
                    .lines()
                    .map(|line| Line::from(line.to_owned()))
                    .collect();
                let cut = vec![false; lines.len()];
                (lines, cut)
            };
            let entry = Entry {
                content: text.to_owned(),
                lines,
                cut,
                width: None,
                heights: Vec::new(),
            };
            if index < self.entries.len() {
                self.entries[index] = entry;
            } else {
                self.entries.push(entry);
            }
        }
        self.entries.truncate(count);
    }

    /// Cached wrapped heights of message `index` at `width`, recomputed after a resize.
    pub fn heights(&mut self, index: usize, width: u16) {
        let entry = &mut self.entries[index];
        if entry.width != Some(width) {
            self.height_passes += 1;
            entry.heights = entry
                .lines
                .iter()
                .zip(&entry.cut)
                .map(|(line, cut)| if *cut { 1 } else { line_height(line, width) })
                .collect();
            entry.width = Some(width);
        }
    }

    pub fn entry(&self, index: usize) -> &Entry {
        &self.entries[index]
    }
}

/// Counters for tests: how often message bodies were rebuilt or re-measured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub builds: usize,
    pub height_passes: usize,
}

#[derive(Default)]
pub struct TranscriptCache {
    inner: RefCell<Inner>,
}

impl TranscriptCache {
    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, Inner> {
        self.inner.borrow_mut()
    }
    pub fn stats(&self) -> Stats {
        let inner = self.inner.borrow();
        Stats {
            builds: inner.builds,
            height_passes: inner.height_passes,
        }
    }
}

// A copy of a session starts cold; the cache never takes part in equality or output.
impl Clone for TranscriptCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl PartialEq for TranscriptCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl std::fmt::Debug for TranscriptCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TranscriptCache")
    }
}
