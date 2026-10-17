//! Per-session cache of rendered chat message bodies and their wrapped heights.
//!
//! `ui::draw` takes `&App`, so the cache sits behind a `RefCell`. An entry is valid only
//! while the message text is byte-identical, so a stale entry cannot be shown. Appending
//! a streamed token changes only the last message and rebuilds only that entry; a resize
//! recomputes heights from the cached lines.
use ratatui::{
    text::Line,
    widgets::{Paragraph, Wrap},
};
use std::cell::RefCell;

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
    /// Make entry `index..` match `messages`, rebuilding only entries whose text changed.
    pub fn sync<'a>(&mut self, messages: impl ExactSizeIterator<Item = &'a str>) {
        let count = messages.len();
        for (index, text) in messages.enumerate() {
            if self.entries.get(index).is_some_and(|e| e.content == text) {
                continue;
            }
            self.builds += 1;
            let entry = Entry {
                content: text.to_owned(),
                lines: crate::dashboard::safe(text)
                    .lines()
                    .map(|line| Line::from(line.to_owned()))
                    .collect(),
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
                .map(|line| line_height(line, width))
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
