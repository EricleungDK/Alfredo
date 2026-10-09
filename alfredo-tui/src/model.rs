use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub const MAX_TEXT: usize = 128 * 1024;
pub const MAX_DRAFT: usize = 16 * 1024;
pub const MAX_SESSIONS: usize = 8;
pub const MAX_MESSAGES: usize = 4096;
/// Notice shown when a paste is cut by [`MAX_DRAFT`]. Typed keys show no notice.
pub const PASTE_TRUNCATED_NOTICE: &str = "Paste truncated to 16 KiB";

/// Outcome of [`Session::insert`]: bytes kept after sanitizing, and whether the
/// 16 KiB draft limit cut the input short.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inserted {
    /// Bytes (not characters) added to the draft.
    pub accepted: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// Presentation provenance; a reference is not task or scope authority.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeReceiptRef {
    pub correlation: String,
    pub revision: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ResponseSource {
    Model { model: String },
    Wayfinder { receipt: Option<ScopeReceiptRef> },
}
impl ResponseSource {
    pub fn label(&self) -> String {
        match self {
            Self::Model { model } => format!("Model · {model}"),
            Self::Wayfinder {
                receipt: Some(receipt),
            } => format!("Wayfinder · scope receipt {}", receipt.revision),
            Self::Wayfinder { receipt: None } => "Wayfinder · no action acknowledged".into(),
        }
    }
    pub(crate) fn valid(&self) -> bool {
        let valid = |text: &str, limit| {
            !text.trim().is_empty() && text.len() <= limit && !text.chars().any(char::is_control)
        };
        match self {
            Self::Model { model } => valid(model, 200),
            Self::Wayfinder {
                receipt: Some(receipt),
            } => (1..=256).contains(&receipt.revision) && valid(&receipt.correlation, 160),
            Self::Wayfinder { receipt: None } => true,
        }
    }
}

/// Local observation placement only; canonical task receipts remain the authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskReceiptRef {
    #[serde(default)]
    pub sequence: u64,
    pub after_messages: usize,
    pub revision: u64,
    pub task: u64,
    pub correlation: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Status {
    Ready,
    Connecting,
    Streaming,
    Complete,
    Cancelled,
    Failed(String),
}

impl Status {
    pub fn active(&self) -> bool {
        matches!(self, Self::Connecting | Self::Streaming)
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Ready => "Ready",
            Self::Connecting => "Preparing request",
            Self::Streaming => "Streaming",
            Self::Complete => "Complete",
            Self::Cancelled => "Cancelled — partial reply retained",
            Self::Failed(_) => "Disconnected / failed",
        }
    }
}

/// Automatic reconnection before any response content: same attempt identity,
/// no side effects. `retry` counts from 1 up to `limit`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Retry {
    pub retry: u32,
    pub limit: u32,
    pub delay: std::time::Duration,
    pub reason: String,
}

#[derive(Debug)]
pub enum Update {
    Metrics(crate::metrics::Metrics),
    Retrying(Retry),
    Queued,
    QueueProgress(crate::inference_admission::Observation),
    /// Another Alfredo process holds the endpoint with a different capacity
    /// (`live`); no ticket is held and nothing is sent until it drains.
    CapacityWait {
        live: usize,
    },
    Admitted,
    Thinking,
    Token(String),
    Done,
    Failed(String),
}

#[derive(Debug)]
pub struct Event {
    pub session: usize,
    pub attempt: u64,
    pub update: Update,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub model: String,
    pub draft: String,
    cursor: usize,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    task_receipts: Vec<TaskReceiptRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    commands: Vec<crate::console_command::ConsoleCommand>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    sources: std::collections::BTreeMap<usize, ResponseSource>,
    pub status: Status,
    pub attempt: u64,
    pub scroll: std::cell::Cell<u16>,
    #[serde(
        default,
        skip_serializing_if = "crate::reading::Viewport::is_unanchored"
    )]
    reading: std::cell::Cell<crate::reading::Viewport>,
    #[serde(skip)]
    queued: bool,
    #[serde(skip)]
    queue_observation: Option<crate::inference_admission::Observation>,
    /// Live capacity of another Alfredo process this request is waiting out.
    #[serde(skip)]
    capacity_wait: Option<usize>,
    #[serde(skip)]
    thinking: bool,
    #[serde(skip)]
    retry: Option<(Retry, std::time::Instant)>,
    #[serde(skip)]
    pub metrics: Option<crate::metrics::Metrics>,
    #[serde(skip)]
    pub timing: Option<crate::client_timing::Timing>,
    #[serde(skip)]
    history: Vec<String>,
    #[serde(skip)]
    history_index: Option<usize>,
    #[serde(skip)]
    unsent: Option<(String, usize)>,
    /// Chat draft set aside while an agent view owns the prompt; snapshots save
    /// this, never the agent's note.
    #[serde(skip)]
    aside: Option<String>,
}

impl Session {
    pub fn new(model: String) -> Self {
        Self {
            model,
            draft: String::new(),
            cursor: 0,
            messages: Vec::new(),
            task_receipts: Vec::new(),
            commands: Vec::new(),
            sources: Default::default(),
            status: Status::Ready,
            attempt: 0,
            scroll: std::cell::Cell::new(0),
            reading: Default::default(),
            queued: false,
            queue_observation: None,
            capacity_wait: None,
            thinking: false,
            retry: None,
            metrics: None,
            timing: None,
            history: Vec::new(),
            history_index: None,
            unsent: None,
            aside: None,
        }
    }

    pub fn status_label(&self) -> std::borrow::Cow<'_, str> {
        if let Some((retry, deadline)) = self.retry.as_ref().filter(|_| self.status.active()) {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            format!(
                "Reconnecting in {}s · retry {}/{}",
                left.as_millis().div_ceil(1000),
                retry.retry,
                retry.limit
            )
            .into()
        } else if let Some(live) = self.capacity_wait.filter(|_| self.status.active()) {
            format!("Waiting for another Alfredo process (capacity {live})").into()
        } else if self.status.active() && self.queued {
            "Queued for Alfredo".into()
        } else if self.status == Status::Connecting && self.thinking {
            "Thinking / waiting for text".into()
        } else if self.status == Status::Connecting
            && self
                .timing
                .as_ref()
                .is_some_and(|timing| timing.has_admission())
        {
            "Waiting for model server".into()
        } else {
            self.status.label().into()
        }
    }

    /// Short phase for the transcript timing line while a request is active.
    pub fn wait_phase(&self) -> Option<&'static str> {
        if !self.status.active() {
            return None;
        }
        if self.retry.is_some() {
            Some("reconnecting")
        } else if self.queued {
            Some("queued")
        } else if self.status == Status::Connecting && self.thinking {
            Some("thinking")
        } else if self.status == Status::Connecting
            && self
                .timing
                .as_ref()
                .is_some_and(|timing| timing.has_admission())
        {
            Some("waiting for model")
        } else {
            None
        }
    }

    /// One short word (or `retry N/M`) for the session list.
    pub fn short_status(&self) -> String {
        if let Some((retry, _)) = self.retry.as_ref().filter(|_| self.status.active()) {
            return format!("retry {}/{}", retry.retry, retry.limit);
        }
        match &self.status {
            Status::Connecting | Status::Streaming if self.queued => "queued".into(),
            Status::Connecting => "thinking".into(),
            Status::Streaming => "streaming".into(),
            Status::Ready => "ready".into(),
            Status::Complete => "done".into(),
            Status::Cancelled => "cancelled".into(),
            Status::Failed(_) => "failed".into(),
        }
    }

    /// Live capacity of another Alfredo process holding the endpoint, while this
    /// request waits for it to drain.
    pub fn capacity_wait(&self) -> Option<usize> {
        self.capacity_wait.filter(|_| self.status.active())
    }

    pub fn queue_observation(&self) -> Option<crate::inference_admission::Observation> {
        (self.status.active() && self.queued)
            .then_some(self.queue_observation)
            .flatten()
    }

    pub fn cursor(&self) -> usize {
        self.draft
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain(std::iter::once(self.draft.len()))
            .take_while(|index| *index <= self.cursor)
            .last()
            .unwrap_or(0)
    }

    pub(crate) fn valid_cursor(&self) -> bool {
        self.cursor == self.cursor()
    }

    pub(crate) fn checkpoint(&self) -> Self {
        let mut snapshot = self.clone();
        if let Some((draft, cursor)) = &self.unsent {
            snapshot.draft = draft.clone();
            snapshot.cursor = *cursor;
        }
        if let Some(chat) = &self.aside {
            snapshot.draft = chat.clone();
            snapshot.cursor = chat.len();
        }
        snapshot.aside = None;
        snapshot.queued = false;
        snapshot.queue_observation = None;
        snapshot.capacity_wait = None;
        snapshot.thinking = false;
        snapshot.retry = None;
        snapshot.timing = None;
        snapshot
            .reading
            .set(self.reading.get().checkpoint(self.scroll.get()));
        snapshot.history.clear();
        snapshot.history_index = None;
        snapshot.unsent = None;
        snapshot
    }
    pub(crate) fn restore_history(&mut self) {
        let prompts: Vec<_> = self
            .messages
            .iter()
            .filter(|m| m.role == "user")
            .map(|m| m.content.clone())
            .collect();
        for text in prompts {
            self.remember(text);
        }
    }

    pub fn remember_submission(&mut self) {
        let text = self.draft.trim().to_string();
        self.remember(text);
    }
    fn remember(&mut self, text: String) {
        if text.is_empty() || text.len() > MAX_DRAFT || self.history.last() == Some(&text) {
            return;
        }
        self.history.push(text);
        while self.history.len() > 100
            || self.history.iter().map(String::len).sum::<usize>() > MAX_TEXT
        {
            self.history.remove(0);
        }
    }
    pub fn history_previous(&mut self) {
        if self.history.is_empty() {
            let prompts: Vec<_> = self
                .messages
                .iter()
                .filter(|m| m.role == "user")
                .map(|m| m.content.clone())
                .collect();
            for text in prompts {
                self.remember(text);
            }
        }
        if self.history.is_empty() {
            return;
        }
        let next = match self.history_index {
            Some(index) => index.saturating_sub(1),
            None => {
                self.unsent = Some((self.draft.clone(), self.cursor()));
                self.history.len() - 1
            }
        };
        self.history_index = Some(next);
        self.draft = self.history[next].clone();
        self.end();
    }
    pub fn history_next(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 < self.history.len() {
            self.history_index = Some(index + 1);
            self.draft = self.history[index + 1].clone();
            self.end();
        } else {
            self.history_index = None;
            if let Some((draft, cursor)) = self.unsent.take() {
                self.draft = draft;
                self.cursor = cursor;
            }
        }
    }

    /// Record the chat draft an agent view set aside (None once it is restored).
    pub fn set_aside_draft(&mut self, chat: Option<String>) {
        self.aside = chat;
    }

    pub fn clear_draft(&mut self) {
        self.history_index = None;
        self.unsent = None;
        self.draft.clear();
        self.cursor = 0;
    }
    pub fn home(&mut self) {
        self.cursor = 0;
    }
    pub fn end(&mut self) {
        self.cursor = self.draft.len();
    }
    pub fn left(&mut self) {
        self.cursor = self.draft[..self.cursor()]
            .grapheme_indices(true)
            .next_back()
            .map(|(index, _)| index)
            .unwrap_or(0);
    }
    pub fn right(&mut self) {
        let cursor = self.cursor();
        self.cursor = cursor
            + self.draft[cursor..]
                .graphemes(true)
                .next()
                .map(str::len)
                .unwrap_or(0);
    }
    pub fn backspace(&mut self) {
        let end = self.cursor();
        self.left();
        self.draft.replace_range(self.cursor..end, "");
    }
    pub fn delete(&mut self) {
        let start = self.cursor();
        self.right();
        self.draft.replace_range(start..self.cursor, "");
        self.cursor = start;
    }
    pub fn delete_word(&mut self) {
        let end = self.cursor();
        let before = &self.draft[..end];
        let trimmed = before.trim_end();
        let start = trimmed
            .char_indices()
            .rev()
            .find(|(_, ch)| ch.is_whitespace())
            .map(|(index, ch)| index + ch.len_utf8())
            .unwrap_or(0);
        self.draft.replace_range(start..end, "");
        self.cursor = start;
    }

    /// Inserts sanitized text at the cursor. Tabs become four spaces, `\r\n` becomes
    /// `\n`, other control characters are dropped, and the 16 KiB limit never splits
    /// a grapheme cluster. The result reports what was kept so callers need not re-derive it.
    pub fn insert(&mut self, text: &str) -> Inserted {
        let cursor = self.cursor();
        let mut clean = String::new();
        let mut truncated = false;
        for grapheme in text.graphemes(true) {
            let mut filtered = String::new();
            for c in grapheme.chars() {
                if c == '\t' {
                    filtered.push_str("    ");
                } else if !c.is_control() || c == '\n' {
                    filtered.push(c);
                }
            }
            if self.draft.len() + clean.len() + filtered.len() > MAX_DRAFT {
                truncated = true;
                break;
            }
            clean.push_str(&filtered);
        }
        let accepted = clean.len();
        self.draft.insert_str(cursor, &clean);
        // Inserting a combining mark/ZWJ can join neighbours into a new cluster.
        self.cursor = self
            .draft
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain(std::iter::once(self.draft.len()))
            .find(|index| *index >= cursor + accepted)
            .unwrap_or(self.draft.len());
        Inserted {
            accepted,
            truncated,
        }
    }

    /// Single-line viewport measured in terminal cells, never UTF-8 bytes.
    pub fn draft_view(&self, width: usize) -> String {
        if width == 0 {
            return String::new();
        }
        let cursor = self.cursor();
        let display = |text: &str| text.replace('\n', "↵");
        let after = display(&self.draft[cursor..]);
        let before = display(&self.draft[..cursor]);
        let right_budget = after.width().min(width / 3);
        let left_budget = width.saturating_sub(1 + right_budget);
        let mut left = Vec::new();
        let mut used = 0;
        for grapheme in before.graphemes(true).rev() {
            if used + grapheme.width() > left_budget {
                break;
            }
            used += grapheme.width();
            left.push(grapheme);
        }
        let mut visible = left.into_iter().rev().collect::<String>();
        visible.push('▏');
        used += 1;
        for grapheme in after.graphemes(true) {
            if used + grapheme.width() > width {
                break;
            }
            used += grapheme.width();
            visible.push_str(grapheme);
        }
        visible
    }

    pub fn begin(&mut self) -> Result<Vec<Message>, &'static str> {
        if self.status.active() {
            return Err("Cancel or wait for the current reply");
        }
        if matches!(self.status, Status::Failed(_) | Status::Cancelled) {
            return Err("Retry the interrupted turn or start a new session");
        }
        let prompt = self.draft.trim().to_string();
        if prompt.is_empty() {
            return Err("Enter a prompt");
        }
        let bytes: usize = self.messages.iter().map(|m| m.content.len()).sum();
        if bytes + prompt.len() > MAX_TEXT || self.messages.len() + 2 > MAX_MESSAGES {
            return Err("Conversation limit reached; start a new session");
        }
        self.remember_submission();
        self.messages.push(Message {
            role: "user".into(),
            content: prompt,
        });
        self.clear_draft();
        Ok(self.start())
    }

    pub fn retry(&mut self) -> Result<Vec<Message>, &'static str> {
        if !matches!(self.status, Status::Failed(_) | Status::Cancelled) {
            return Err("Only interrupted turns can be retried");
        }
        // The incomplete assistant message stays visible until an explicit retry.
        self.messages.pop();
        self.sources.remove(&self.messages.len());
        Ok(self.start())
    }

    fn start(&mut self) -> Vec<Message> {
        let request = self.messages.clone();
        self.messages.push(Message {
            role: "assistant".into(),
            content: String::new(),
        });
        self.attempt += 1;
        self.queued = false;
        self.queue_observation = None;
        self.capacity_wait = None;
        self.thinking = false;
        self.retry = None;
        self.metrics = None;
        self.timing = Some(crate::client_timing::Timing::new(std::time::Instant::now()));
        self.status = Status::Connecting;
        self.scroll.set(0);
        self.reading.set(Default::default());
        request
    }

    pub fn cancel(&mut self) {
        if self.status.active() {
            self.status = Status::Cancelled;
            self.retry = None;
            self.queued = false;
            self.queue_observation = None;
            self.capacity_wait = None;
            if let Some(timing) = &mut self.timing {
                timing.finish(std::time::Instant::now());
            }
        }
    }

    pub fn scroll_rows(&self, rows: i32) {
        let mut reading = self.reading.get();
        reading.move_rows(rows);
        self.reading.set(reading);
    }
    pub fn reading_position(&self, heights: &[usize], height: u16) -> crate::reading::Position {
        let mut reading = self.reading.get();
        let position = reading.position(&self.scroll, heights, height);
        self.reading.set(reading);
        position
    }
    pub fn reading_position_blocks(
        &self,
        heights: &[usize],
        height: u16,
        blocks: &[crate::reading::Block],
    ) -> crate::reading::Position {
        let mut reading = self.reading.get();
        let position = reading.position_blocks(&self.scroll, heights, height, blocks);
        self.reading.set(reading);
        position
    }
    fn next_console_sequence(&self) -> u64 {
        self.task_receipts
            .iter()
            .map(|r| r.sequence)
            .chain(self.commands.iter().map(|c| c.sequence))
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }
    pub fn commands(&self) -> &[crate::console_command::ConsoleCommand] {
        &self.commands
    }
    pub fn submit_command(
        &mut self,
        text: String,
        intent: crate::command_intent::Intent,
    ) -> Result<String, String> {
        self.admit_command(
            text,
            intent,
            true,
            None,
            crate::console_command::CommandState::Pending,
        )
    }
    /// Record an autopilot-chosen command like typed input, but keep the reader's
    /// position: autopilot must not pull a user who scrolled up to the bottom.
    pub fn submit_autopilot_command(
        &mut self,
        text: String,
        intent: crate::command_intent::Intent,
    ) -> Result<String, String> {
        self.admit_command(
            text,
            intent,
            false,
            None,
            crate::console_command::CommandState::Pending,
        )
    }
    /// Append a controller-selected launch at its source without moving the reader
    /// or changing the composer's unfinished work.
    pub fn submit_automatic_command(
        &mut self,
        text: String,
        intent: crate::command_intent::Intent,
    ) -> Result<String, String> {
        if !matches!(
            intent,
            crate::command_intent::Intent::DispatchRun { .. }
                | crate::command_intent::Intent::ArchitectDraft { .. }
        ) {
            return Err("Automatic entries require a controller-selected intent".into());
        }
        self.admit_command(
            text,
            intent,
            false,
            None,
            crate::console_command::CommandState::Pending,
        )
    }
    /// Keep a delayed scope operation with its exact existing user turn. The
    /// compact command is presentation history, never an extra model message.
    pub fn submit_wayfinder_command(
        &mut self,
        user_message: usize,
        request: crate::understanding::Request,
    ) -> Result<String, String> {
        let after_messages = user_message
            .checked_add(2)
            .ok_or("Wayfinder turn is out of bounds")?;
        if !user_message.is_multiple_of(2)
            || after_messages > self.messages.len()
            || !crate::wayfinder::request_matches_prompt(
                &request,
                &self.messages[user_message].content,
            )
        {
            return Err("Wayfinder request does not match its saved user turn".into());
        }
        let text = match request.action {
            crate::understanding::Action::Enter { .. } => "Enter shared understanding".into(),
            crate::understanding::Action::Draft { .. } => "Save scope draft for review".into(),
            crate::understanding::Action::Confirm { draft_revision } => {
                format!("Confirm shared understanding r{draft_revision}")
            }
        };
        self.admit_command(
            text,
            crate::command_intent::Intent::Wayfinder {
                request,
                user_message,
            },
            false,
            Some(after_messages),
            crate::console_command::CommandState::Pending,
        )
    }
    pub fn submit_selection_command(
        &mut self,
        text: String,
        request: crate::selection_command::Request,
    ) -> Result<String, String> {
        self.admit_command(
            text,
            crate::command_intent::Intent::Selection { request },
            false,
            None,
            crate::console_command::CommandState::Pending,
        )
    }
    pub fn submit_selection_arrival(
        &mut self,
        request: crate::selection_command::Request,
        outcome: crate::selection_command::Outcome,
    ) -> Result<String, String> {
        self.admit_command(
            format!(
                "Workspace {} · mission {}",
                request.choice.target().display(),
                request.choice.mission.name()
            ),
            crate::command_intent::Intent::SelectionArrival { request },
            false,
            None,
            crate::console_command::CommandState::Selection { outcome },
        )
    }
    fn admit_command(
        &mut self,
        text: String,
        intent: crate::command_intent::Intent,
        reveal: bool,
        after_messages: Option<usize>,
        state: crate::console_command::CommandState,
    ) -> Result<String, String> {
        use crate::console_command::ConsoleCommand;
        let command = ConsoleCommand {
            id: ConsoleCommand::identity(&intent),
            sequence: self.next_console_sequence(),
            after_messages: after_messages.unwrap_or(self.messages.len()),
            attempt: 1,
            text,
            intent,
            state,
        };
        if self.commands.iter().any(|old| old.id == command.id) {
            return Err("Retry the original command entry".into());
        }
        if !command.valid(self.messages.len()) {
            return Err("Invalid command intent".into());
        }
        self.commands.push(command);
        if !self.valid_commands(17) {
            self.commands.pop();
            return Err("Command history capacity reached; start another conversation".into());
        }
        if reveal {
            self.scroll.set(0);
            self.reading.set(Default::default());
        }
        Ok(self.commands.last().unwrap().id.clone())
    }
    pub fn set_command_state(
        &mut self,
        id: &str,
        state: crate::console_command::CommandState,
    ) -> bool {
        let Some(command) = self.commands.iter_mut().find(|command| command.id == id) else {
            return false;
        };
        command.state = state;
        true
    }
    pub fn retry_command(&mut self, id: &str) -> Result<(), String> {
        let command = self
            .commands
            .iter_mut()
            .find(|command| command.id == id)
            .ok_or("Command origin unavailable")?;
        if command.intent.selection_request().is_some() {
            return Err(
                "Selection requires journal reconciliation; use /workspace for a new choice".into(),
            );
        }
        if matches!(
            command.intent,
            crate::command_intent::Intent::DispatchRun { .. }
        ) {
            return Err("Automatic launch is historical; submit /run for an explicit retry".into());
        }
        if matches!(
            command.intent,
            crate::command_intent::Intent::ArchitectDraft { .. }
        ) {
            return Err("Automatic Architect draft is historical; submit /architect-revise for an explicit retry".into());
        }
        if matches!(
            command.state,
            crate::console_command::CommandState::Planner { .. }
                | crate::console_command::CommandState::Control { .. }
        ) {
            return Err("Command has a recorded local outcome; submit a new command".into());
        }
        command.attempt = command
            .attempt
            .checked_add(1)
            .filter(|n| *n < u64::MAX)
            .ok_or("Retry limit reached")?;
        command.state = crate::console_command::CommandState::Pending;
        Ok(())
    }
    pub(crate) fn restore_commands(&mut self) {
        use crate::console_command::CommandState;
        for command in &mut self.commands {
            if matches!(
                command.state,
                CommandState::Pending | CommandState::Submitted
            ) {
                command.state = CommandState::Unknown {
                    reason: if command.intent.selection_request().is_some() {
                        "Interrupted selection; inspect journal and use /workspace for a new choice"
                    } else {
                        "Interrupted command; reconcile receipts before explicit retry"
                    }
                    .into(),
                };
            }
        }
    }
    pub(crate) fn valid_commands(&self, schema: u32) -> bool {
        use std::collections::BTreeSet;
        if (schema < 10 && !self.commands.is_empty())
            || (schema < 12
                && self.commands.iter().any(|command| {
                    matches!(
                        command.intent,
                        crate::command_intent::Intent::Planner { .. }
                    ) || matches!(
                        command.state,
                        crate::console_command::CommandState::Planner { .. }
                    )
                }))
            || (schema < 13
                && self.commands.iter().any(|command| {
                    matches!(
                        command.intent,
                        crate::command_intent::Intent::Control { .. }
                    ) || matches!(
                        command.state,
                        crate::console_command::CommandState::Control { .. }
                    )
                }))
            || (schema < 14
                && self.commands.iter().any(|command| {
                    matches!(
                        command.intent,
                        crate::command_intent::Intent::DispatchRun { .. }
                    )
                }))
            || (schema < 15
                && self.commands.iter().any(|command| {
                    matches!(
                        command.intent,
                        crate::command_intent::Intent::ArchitectDraft { .. }
                    )
                }))
            || (schema < 16
                && self.commands.iter().any(|command| {
                    matches!(
                        command.intent,
                        crate::command_intent::Intent::Wayfinder { .. }
                    )
                }))
            || (schema < 17
                && self.commands.iter().any(|command| {
                    command.intent.selection_request().is_some()
                        || matches!(
                            command.state,
                            crate::console_command::CommandState::Selection { .. }
                        )
                }))
            || self.commands.len() > 512
            || self
                .commands
                .iter()
                .any(|c| !c.valid(self.messages.len()) || c.sequence > 8192)
            || self
                .task_receipts
                .iter()
                .any(|reference| reference.sequence > 8192)
        {
            return false;
        }
        let mut ids = BTreeSet::new();
        let mut planner_ids = BTreeSet::new();
        let mut scope_ids = BTreeSet::new();
        let mut selection_ids = BTreeSet::new();
        let mut wayfinder_turns = BTreeSet::new();
        let mut sequences = BTreeSet::new();
        for reference in &self.task_receipts {
            if reference.sequence > 0 && !sequences.insert(reference.sequence) {
                return false;
            }
        }
        for command in &self.commands {
            if let Some(request) = command.intent.selection_request() {
                if !selection_ids.insert(&request.correlation) {
                    return false;
                }
            }
            if let Some(request) = command.intent.scope_request() {
                if !scope_ids.insert(&request.correlation) {
                    return false;
                }
            }
            if let crate::command_intent::Intent::Wayfinder {
                request,
                user_message,
            } = &command.intent
            {
                if !user_message.is_multiple_of(2)
                    || user_message.checked_add(2) != Some(command.after_messages)
                    || self.messages.get(*user_message).is_none_or(|message| {
                        message.role != "user"
                            || !crate::wayfinder::request_matches_prompt(request, &message.content)
                    })
                    || self
                        .messages
                        .get(user_message.saturating_add(1))
                        .is_none_or(|message| message.role != "assistant")
                    || !wayfinder_turns.insert(*user_message)
                {
                    return false;
                }
            }
            if let Some(request) = command.intent.planner_request() {
                if !planner_ids.insert(&request.correlation) {
                    return false;
                }
            }
            if let crate::command_intent::Intent::ArchitectDraft { request } = &command.intent {
                if !self.commands.iter().any(|parent| {
                    parent.sequence < command.sequence
                        && matches!(&parent.intent, crate::command_intent::Intent::Task { request: source } if source == &request.source)
                }) {
                    return false;
                }
            }
            if let crate::command_intent::Intent::DispatchRun { request } = &command.intent {
                let parent = self.commands.iter().any(|parent| {
                    parent.sequence < command.sequence
                        && matches!(&parent.intent, crate::command_intent::Intent::Control { request: source } if source == &request.source)
                        && matches!(parent.state, crate::console_command::CommandState::Control {
                            outcome: crate::control_command::Outcome::DispatchChanged { enabled: true }
                        })
                });
                if !parent {
                    return false;
                }
            }
            if !ids.insert(&command.id) || !sequences.insert(command.sequence) {
                return false;
            }
        }
        if !self
            .commands
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
        {
            return false;
        }
        let chronological: Vec<_> = self
            .commands
            .iter()
            .filter(|command| {
                schema < 16
                    || !matches!(
                        command.intent,
                        crate::command_intent::Intent::Wayfinder { .. }
                    )
            })
            .collect();
        if !chronological
            .windows(2)
            .all(|pair| pair[0].after_messages <= pair[1].after_messages)
        {
            return false;
        }
        if !self.task_receipts.windows(2).all(|pair| {
            pair[1].sequence > pair[0].sequence || pair[0].sequence == 0 && pair[1].sequence == 0
        }) {
            return false;
        }
        // Reserve terminal metadata once against an immutable normalized base.
        let mut budgeted = self.commands.clone();
        for command in &mut budgeted {
            command.state = crate::console_command::CommandState::Pending;
            command.attempt = 1;
        }
        let bytes = serde_json::to_vec(&budgeted).map_or(usize::MAX, |bytes| bytes.len());
        bytes.saturating_add(self.commands.len() * 1024) <= 512 * 1024
    }
    pub fn task_receipts(&self) -> &[TaskReceiptRef] {
        &self.task_receipts
    }
    pub fn observe_task_receipt(&mut self, mut reference: TaskReceiptRef) -> bool {
        if self.status.active()
            || self.task_receipts.len() >= 4096
            || !self.valid_task_reference(&reference)
            || self.task_receipts.iter().any(|old| {
                old.revision >= reference.revision
                    || old.correlation == reference.correlation
                    || old.after_messages > reference.after_messages
            })
        {
            return false;
        }
        reference.sequence = self.next_console_sequence();
        if reference.sequence > 8192 {
            return false;
        }
        self.task_receipts.push(reference);
        true
    }
    fn valid_task_reference(&self, reference: &TaskReceiptRef) -> bool {
        reference.after_messages <= self.messages.len()
            && reference.after_messages.is_multiple_of(2)
            && (1..=4096).contains(&reference.revision)
            && (1..=256).contains(&reference.task)
            && !reference.correlation.trim().is_empty()
            && reference.correlation.len() <= 160
            && !reference.correlation.chars().any(char::is_control)
    }
    pub(crate) fn valid_task_receipts(&self, schema: u32) -> bool {
        (schema >= 8 || self.task_receipts.is_empty())
            && (schema >= 10
                || self
                    .task_receipts
                    .iter()
                    .all(|reference| reference.sequence == 0))
            && self.task_receipts.len() <= 4096
            && self
                .task_receipts
                .iter()
                .all(|reference| self.valid_task_reference(reference))
            && self.task_receipts.windows(2).all(|pair| {
                pair[0].revision < pair[1].revision
                    && pair[0].after_messages <= pair[1].after_messages
            })
            && self
                .task_receipts
                .iter()
                .map(|reference| &reference.correlation)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == self.task_receipts.len()
    }
    pub fn source(&self, message: usize) -> Option<&ResponseSource> {
        self.sources.get(&message)
    }
    pub(crate) fn valid_reading(&self, schema: u32) -> bool {
        // Newline plus two structural lines per message is a conservative bound;
        // layout clamps the wrapped row when terminal geometry changes.
        let lines = if self.messages.is_empty() {
            3
        } else {
            self.messages
                .iter()
                .map(|m| m.content.bytes().filter(|b| *b == b'\n').count() + 3)
                .sum()
        };
        let reading = self.reading.get();
        let has_worker_result = |command: &crate::console_command::ConsoleCommand| {
            (schema >= 11 && matches!(command.intent, crate::command_intent::Intent::Run { .. }))
                || (schema >= 14
                    && matches!(
                        command.intent,
                        crate::command_intent::Intent::DispatchRun { .. }
                    ))
                || (schema >= 13
                    && matches!(&command.intent, crate::command_intent::Intent::Control { request } if matches!(request.operation, crate::control_command::Operation::CancelWorker { .. })))
        };
        let valid_block = reading.block_anchor().is_none_or(|(key, line)| match key {
            crate::reading::BlockKey::Message(index) => {
                self.messages.get(index).is_some_and(|message| {
                    line < message.content.bytes().filter(|b| *b == b'\n').count() + 3
                })
            }
            crate::reading::BlockKey::Command(sequence) => self.commands.iter().any(|command| {
                command.sequence == sequence
                    && line
                        < command.text.bytes().filter(|b| *b == b'\n').count()
                            + 4
                            + usize::from(has_worker_result(command))
            }),
            crate::reading::BlockKey::TaskReceipt(revision) => {
                line < 3
                    && self
                        .task_receipts
                        .iter()
                        .any(|reference| reference.revision == revision)
            }
        });
        valid_block
            && reading.valid(
                schema,
                self.scroll.get(),
                lines
                    + 3 * self.task_receipts.len()
                    + self
                        .commands
                        .iter()
                        .map(|command| {
                            command.text.bytes().filter(|b| *b == b'\n').count()
                                + 4
                                + usize::from(has_worker_result(command))
                        })
                        .sum::<usize>(),
                MAX_TEXT + 200,
            )
    }
    pub(crate) fn valid_sources(&self, schema: u32) -> bool {
        (schema >= 3 || self.sources.is_empty())
            && self.sources.iter().all(|(index, source)| {
                index % 2 == 1 && *index < self.messages.len() && source.valid()
            })
    }
    pub fn wayfinder_reply(
        &mut self,
        attempt: u64,
        text: String,
        receipt: Option<ScopeReceiptRef>,
    ) {
        if self.attempt != attempt || !self.status.active() || self.messages.is_empty() {
            return;
        }
        let index = self.messages.len() - 1;
        if self.sources.contains_key(&index) {
            return;
        }
        let source = ResponseSource::Wayfinder { receipt };
        if !source.valid() {
            return;
        }
        self.sources.insert(index, source);
        self.apply(attempt, Update::Token(text));
        self.apply(attempt, Update::Done);
    }
    pub fn apply(&mut self, attempt: u64, update: Update) {
        if self.attempt != attempt || !self.status.active() {
            return;
        }
        if matches!(
            update,
            Update::Queued
                | Update::QueueProgress(_)
                | Update::CapacityWait { .. }
                | Update::Admitted
                | Update::Thinking
                | Update::Metrics(_)
                | Update::Token(_)
        ) {
            self.sources
                .entry(self.messages.len() - 1)
                .or_insert_with(|| ResponseSource::Model {
                    model: self.model.clone(),
                });
        }
        if !matches!(update, Update::Retrying(_) | Update::Metrics(_)) {
            self.retry = None;
        }
        match update {
            Update::Metrics(metrics) => self.metrics = Some(metrics),
            Update::Retrying(retry) => {
                if self.status == Status::Connecting {
                    let deadline = std::time::Instant::now() + retry.delay;
                    self.retry = Some((retry, deadline));
                    self.queued = false;
                    self.queue_observation = None;
                    self.capacity_wait = None;
                }
            }
            Update::Queued | Update::QueueProgress(_) | Update::CapacityWait { .. }
                if self.status != Status::Connecting
                    || self.thinking
                    || self
                        .timing
                        .as_ref()
                        .is_some_and(|timing| timing.has_admission()) => {}
            Update::CapacityWait { live } => {
                self.queued = true;
                self.queue_observation = None;
                self.capacity_wait = Some(live);
            }
            Update::Queued => {
                self.queued = true;
                self.capacity_wait = None;
            }
            Update::QueueProgress(observation) => {
                self.queued = true;
                self.capacity_wait = None;
                self.queue_observation = Some(observation);
            }
            Update::Thinking => {
                self.thinking = true;
                self.queued = false;
                self.queue_observation = None;
                self.capacity_wait = None;
            }
            Update::Admitted => {
                self.queued = false;
                self.queue_observation = None;
                self.capacity_wait = None;
                if let Some(timing) = &mut self.timing {
                    timing.admit(std::time::Instant::now());
                }
            }
            Update::Token(text) => {
                if !text.is_empty() {
                    if let Some(timing) = &mut self.timing {
                        timing.text(std::time::Instant::now());
                    }
                }
                self.queued = false;
                self.queue_observation = None;
                self.capacity_wait = None;
                let total: usize = self.messages.iter().map(|m| m.content.len()).sum();
                if total + text.len() > MAX_TEXT {
                    self.status = Status::Failed("Conversation output limit reached".into());
                } else {
                    self.messages.last_mut().unwrap().content.push_str(&text);
                    self.status = Status::Streaming;
                }
            }
            Update::Done => self.status = Status::Complete,
            Update::Failed(reason) => self.status = Status::Failed(reason),
        }
        if !self.status.active() {
            self.retry = None;
            self.queued = false;
            self.queue_observation = None;
            self.capacity_wait = None;
            if let Some(timing) = &mut self.timing {
                timing.finish(std::time::Instant::now());
            }
        }
    }
}

pub struct App {
    pub sessions: Vec<Session>,
    pub selected: usize,
    pub notice: String,
    pub models: Vec<String>,
    pub models_visible: bool,
    pub models_pending: bool,
    pub models_notice: String,
    pub models_scroll: u16,
    /// Catalog row the model list's arrow keys point at.
    pub models_cursor: usize,
    pub completion: Option<crate::commands::Completion>,
    /// Transient server health; inert until a workstation starts its monitor.
    pub health: crate::health::HealthView,
    /// Side pane focus, cursor, presentation settings and mission list (view state only).
    pub pane: crate::side_pane::PaneUi,
}

impl App {
    pub fn new(model: String) -> Self {
        Self {
            sessions: vec![Session::new(model)],
            selected: 0,
            notice: String::new(),
            models: Vec::new(),
            models_visible: false,
            models_pending: false,
            models_notice: String::new(),
            models_scroll: 0,
            models_cursor: 0,
            completion: None,
            health: Default::default(),
            pane: Default::default(),
        }
    }

    pub fn receive_models(&mut self, result: Result<Vec<String>, String>) {
        self.models_pending = false;
        match result {
            Ok(models) => {
                self.models = models;
                let current = &self.sessions[self.selected].model;
                self.models_cursor = self
                    .models
                    .iter()
                    .position(|model| model == current)
                    .unwrap_or(0);
                self.models_notice = if self.models.is_empty() {
                    "No installed models reported".into()
                } else {
                    "Installed models · ↑↓ Enter or /model NAME selects for this chat".into()
                };
            }
            Err(error) => {
                self.models_notice = format!("{error}; previous catalog retained · /models retries")
            }
        }
    }

    pub fn move_model_cursor(&mut self, down: bool) {
        self.models_cursor = if down {
            (self.models_cursor + 1).min(self.models.len().saturating_sub(1))
        } else {
            self.models_cursor.saturating_sub(1)
        };
    }

    pub fn choose_model(&mut self) -> Result<(), &'static str> {
        let name = self
            .models
            .get(self.models_cursor)
            .cloned()
            .ok_or("No installed models to choose")?;
        self.select_model(&name)
    }

    pub fn select_model(&mut self, name: &str) -> Result<(), &'static str> {
        if self.models_pending {
            return Err("Wait for model discovery to finish");
        }
        if !self.models.iter().any(|model| model == name) {
            return Err("Model not in the catalog; /models refreshes installed models");
        }
        let session = &mut self.sessions[self.selected];
        if session.status.active()
            || matches!(session.status, Status::Failed(_) | Status::Cancelled)
        {
            return Err("Active or interrupted turns retain their model; finish/retry or open a new conversation");
        }
        session.model = name.into();
        self.health.preload(name);
        self.models_visible = false;
        self.notice =
            format!("Conversation model: {name}. Existing task assignments are unchanged");
        Ok(())
    }

    pub fn add_session(&mut self) {
        if self.sessions.len() == MAX_SESSIONS {
            self.notice = "Eight-session limit reached".into();
            return;
        }
        let model = self.sessions[self.selected].model.clone();
        self.sessions.push(Session::new(model));
        self.selected = self.sessions.len() - 1;
        self.notice.clear();
    }

    pub fn apply(&mut self, event: Event) {
        if let Some(session) = self.sessions.get_mut(event.session) {
            session.apply(event.attempt, event.update);
        }
    }
}
