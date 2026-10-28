use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[path = "compaction.rs"]
pub mod compaction;
pub use compaction::Compaction;

pub const MAX_TEXT: usize = 128 * 1024;
/// Failure for a reply that finished with no text (issue #397).
pub const NO_TEXT: &str = "Model returned no text; Ctrl+R retries or send again";
/// Failure text when a reply or tool step would pass [`MAX_TEXT`]; names the ways out.
pub const OUTPUT_LIMIT: &str = "Conversation output limit reached; run /compact or /clear, or press Ctrl+N for a new conversation";
pub const MAX_DRAFT: usize = 16 * 1024;
const COMMAND_BUDGET_COUNT: usize = 512;
const COMMAND_BUDGET_BYTES: usize = 512 * 1024;
/// Largest console sequence a saved command or task receipt may carry.
const MAX_CONSOLE_SEQUENCE: u64 = 8192;
/// How many evicted commands' ids and correlations are remembered; older ones are
/// forgotten, and the stores' own duplicate guards remain the backstop.
const RETIRED_COMMANDS: usize = 1024;
const CAPACITY: &str = "Command history capacity reached; start another conversation";
/// Normalized command JSON plus 1 KiB reserved per command for terminal metadata.
fn command_budget_use(commands: &[crate::console_command::ConsoleCommand]) -> usize {
    budget_bytes(
        commands.len(),
        command_sizes(commands)
            .iter()
            .fold(0, |sum, size| sum.saturating_add(*size)),
    )
}
/// The JSON array of `count` commands whose sized lengths add up to `sum`: brackets and
/// one comma between neighbours.
fn budget_bytes(count: usize, sum: usize) -> usize {
    if count == 0 {
        2
    } else {
        sum.saturating_add(count + 1)
    }
}
/// What an evicted command keeps refused: its id first, then each correlation the
/// structure rules keep unique among evictable commands (selection commands are never
/// evicted).
fn retirement_keys(command: &crate::console_command::ConsoleCommand) -> Vec<String> {
    let mut keys = vec![format!("id:{}", command.id)];
    let intent = &command.intent;
    if let Some(request) = intent.scope_request() {
        keys.push(format!("scope:{}", request.correlation));
    }
    if let Some(request) = intent.planner_request() {
        keys.push(format!("planner:{}", request.correlation));
    }
    keys
}

/// Each command's normalized JSON length plus its 1 KiB reservation, serialized once.
fn command_sizes(commands: &[crate::console_command::ConsoleCommand]) -> Vec<usize> {
    commands
        .iter()
        .map(|command| {
            let mut budgeted = command.clone();
            budgeted.state = crate::console_command::CommandState::Pending;
            budgeted.attempt = 1;
            serde_json::to_vec(&budgeted)
                .map_or(usize::MAX, |bytes| bytes.len())
                .saturating_add(1024)
        })
        .collect()
}
/// Messages that can wait behind a running turn, and their total size (issue #338).
pub const MAX_QUEUED: usize = 3;
const MAX_QUEUED_BYTES: usize = MAX_DRAFT / 2;
const BUSY: &str = "Esc stops the reply, or wait for it";
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

/// Internal role of a stored tool-call request: `content` is the JSON array of
/// [`ToolCall`]s. The provider sends it as an assistant message with `tool_calls`;
/// results follow as `tool` messages, one per call, in call order (issue #233).
pub const TOOL_CALLS_ROLE: &str = "tool_calls";

/// One function call the model asked for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

/// One round of the chat tool loop: the calls of one model reply and their results
/// (same order, same length).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolStep {
    pub calls: Vec<ToolCall>,
    pub results: Vec<String>,
}
impl ToolStep {
    /// Bytes this step adds to a request (calls as JSON plus results).
    pub fn bytes(&self) -> usize {
        serde_json::to_string(&self.calls).map_or(0, |text| text.len())
            + self.results.iter().map(String::len).sum::<usize>()
    }
    /// Model-visible form: the call request, then one `tool` message per result.
    pub fn messages(&self) -> impl Iterator<Item = Message> + '_ {
        std::iter::once(Message {
            role: TOOL_CALLS_ROLE.into(),
            content: serde_json::to_string(&self.calls).unwrap_or_else(|_| "[]".into()),
        })
        .chain(self.results.iter().map(|result| Message {
            role: "tool".into(),
            content: result.clone(),
        }))
    }
    pub(crate) fn valid(&self) -> bool {
        !self.calls.is_empty()
            && self.calls.len() == self.results.len()
            && self.calls.len() <= 64
            && self.calls.iter().all(|call| {
                !call.name.is_empty()
                    && call.name.len() <= 64
                    && !call.name.chars().any(char::is_control)
            })
    }
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
    /// The model asked for tools (provider; consumed by the chat tool loop).
    ToolCalls(Vec<ToolCall>),
    /// A tool round finished: calls and results join the reply's context.
    ToolStep(ToolStep),
    /// The chat tool loop ended the turn early (a cap, not the model's answer). Sent
    /// before the notice text and `Done`/`Failed`, so headless runs can tell it apart.
    Stopped(Stop),
    /// One call of a tool round finished (issue #271): it joins the transcript and
    /// context at once, so a turn cut short later keeps what already ran.
    /// `new_round` opens a new [`ToolStep`]; otherwise it extends the last one.
    ToolResult {
        call: ToolCall,
        result: String,
        new_round: bool,
    },
    /// A tool call is about to run because always-allow rule `rule` approved it, with no
    /// card (issue #320). Display only: marks the call's row and, for `run_command`,
    /// the `running run_command` status.
    RuleRunning {
        tool: String,
        rule: u64,
    },
    /// A line for the user about the turn (issue #281), e.g. a model that has no tool
    /// support. Shown as a notice (TUI footer, headless stderr); never stored in the
    /// assistant message, so it is not sent back to the model.
    Notice(String),
    Done,
    Failed(String),
}

/// Why a chat turn was cut off by a cap (issue #278).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The tool step cap was reached.
    StepLimit,
    /// The conversation had no room left for another round.
    OutputLimit,
}

impl Stop {
    /// The `stopped` value of the headless chat JSON.
    pub fn label(self) -> &'static str {
        match self {
            Self::StepLimit => "step_limit",
            Self::OutputLimit => "output_limit",
        }
    }
}

#[derive(Debug)]
pub struct Event {
    pub session: usize,
    pub attempt: u64,
    pub update: Update,
}

/// The pending plan draft's origin request, held by the session that may own that command.
/// It is derived runtime state, so it never takes part in session equality.
#[derive(Clone, Debug, Default)]
struct PlanOrigin(Option<crate::planner_command::Request>);

impl PartialEq for PlanOrigin {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub model: String,
    pub draft: String,
    cursor: usize,
    pub messages: Vec<Message>,
    /// Note that replaced the oldest turns (issue #187); see [`compaction`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compaction: Option<Compaction>,
    /// A summary request is in flight: the draft it was started with, and whether to send it after.
    #[serde(skip)]
    compacting: Option<(String, bool)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    task_receipts: Vec<TaskReceiptRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    commands: Vec<crate::console_command::ConsoleCommand>,
    /// Ids and correlations of evicted commands (newest last, at most
    /// `RETIRED_COMMANDS`), so an eviction does not make a repeat admissible.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    retired_commands: Vec<String>,
    /// Origin of the pending plan draft (issue #475): the draft is saved with this command,
    /// whatever its state, so eviction must keep it. Runtime only, never saved.
    #[serde(skip)]
    plan_origin: PlanOrigin,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    sources: std::collections::BTreeMap<usize, ResponseSource>,
    /// User-message indices of cancelled or failed turns the user moved past with
    /// a new prompt. They stay in the transcript but are not sent to the model.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    interrupted: Vec<usize>,
    pub status: Status,
    pub attempt: u64,
    /// `@path` file contents (working tree) appended to a user turn's model-visible
    /// message, keyed by user-message index. Not shown, not saved.
    #[serde(skip)]
    attachments: std::collections::BTreeMap<usize, String>,
    /// Tool rounds of each reply, keyed by assistant-message index (issue #233).
    /// Sent before that reply as context; saved (schema v19).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    tool_steps: std::collections::BTreeMap<usize, Vec<ToolStep>>,
    /// Always-allow rule that approved each call, keyed by assistant-message index then
    /// the call's position among that reply's calls (issue #320). Display only; not saved.
    #[serde(skip)]
    rule_marks: std::collections::BTreeMap<usize, Vec<(usize, u64)>>,
    /// Rule of the call that is running now, taken by its result.
    #[serde(skip)]
    pending_rule: Option<u64>,
    /// Project-notes arrival lines (issue #322) and the content fingerprint last
    /// announced. Display only: never sent to the model, never saved.
    #[serde(skip)]
    notes_lines: Vec<String>,
    #[serde(skip)]
    notes_fingerprint: Option<u64>,
    /// The user message of the latest prompt with refused `@path` mentions and one row
    /// per refusal (issue #391). Display only: never sent to the model, never saved.
    #[serde(skip)]
    mention_refusals: (usize, Vec<String>),
    pub scroll: std::cell::Cell<u16>,
    #[serde(
        default,
        skip_serializing_if = "crate::reading::Viewport::is_unanchored"
    )]
    reading: std::cell::Cell<crate::reading::Viewport>,
    #[serde(skip)]
    pub(crate) transcript_cache: crate::transcript_cache::TranscriptCache,
    /// Show every tool result in full in the transcript (Ctrl+O; issue #258). View
    /// state only: not saved, and a restored conversation starts collapsed.
    #[serde(skip)]
    pub tool_results_expanded: bool,
    #[serde(skip)]
    queued: bool,
    /// Messages entered while a turn runs (issue #338): shown as `You · queued`, sent in
    /// order once the turn completes, and returned to the prompt as a draft when it is
    /// cancelled or fails. Not saved: a restart restores them as the draft (see
    /// [`Session::checkpoint`]). Unrelated to `queued`, which is waiting for admission.
    #[serde(skip)]
    queue: Vec<String>,
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
    /// Draft text a rejected command left in the prompt; the next typed character
    /// replaces it unless an editing or cursor key claimed it first.
    #[serde(skip)]
    rejected: Option<String>,
    /// An approved `run_command` is running (issue #302); cleared by the next update.
    #[serde(skip)]
    running_command: bool,
    /// Chat draft set aside while an agent view owns the prompt; snapshots save
    /// this, never the agent's note.
    #[serde(skip)]
    aside: Option<String>,
    /// A Chat Agent action waiting for the owner (issue #235). Never saved, so a
    /// restored session cannot re-ask or run it.
    #[serde(skip)]
    /// The card and when it became visible (None while not on screen).
    approval: Option<(
        crate::chat_tools::approval::Card,
        Option<std::time::Instant>,
    )>,
    /// Set by the last draw: the pending card's header and args rows were inside
    /// the transcript viewport (issue #268). Arming waits for it.
    #[serde(skip)]
    approval_header_drawn: std::cell::Cell<bool>,
    /// The last draw put the whole card, every command line included, in the viewport.
    /// Only then may `y` or `a` answer it (issue #303); `n` needs just the header.
    #[serde(skip)]
    approval_whole: std::cell::Cell<bool>,
    /// When the key before the current one, and the current one, were pressed
    /// (issue #264: a y/n only answers a card after a quiet gap).
    #[serde(skip)]
    key_times: (Option<std::time::Instant>, Option<std::time::Instant>),
}

impl Session {
    pub fn new(model: String) -> Self {
        Self {
            model,
            draft: String::new(),
            cursor: 0,
            messages: Vec::new(),
            compaction: None,
            compacting: None,
            task_receipts: Vec::new(),
            commands: Vec::new(),
            retired_commands: Vec::new(),
            plan_origin: PlanOrigin::default(),
            sources: Default::default(),
            interrupted: Vec::new(),
            status: Status::Ready,
            attempt: 0,
            attachments: Default::default(),
            tool_steps: Default::default(),
            rule_marks: Default::default(),
            pending_rule: None,
            notes_lines: Vec::new(),
            notes_fingerprint: None,
            mention_refusals: (0, Vec::new()),
            scroll: std::cell::Cell::new(0),
            reading: Default::default(),
            transcript_cache: Default::default(),
            tool_results_expanded: false,
            queued: false,
            queue: Vec::new(),
            queue_observation: None,
            capacity_wait: None,
            thinking: false,
            retry: None,
            metrics: None,
            timing: None,
            history: Vec::new(),
            history_index: None,
            unsent: None,
            rejected: None,
            running_command: false,
            approval: None,
            approval_header_drawn: std::cell::Cell::new(false),
            approval_whole: std::cell::Cell::new(false),
            key_times: (None, None),
            aside: None,
        }
    }

    pub fn status_label(&self) -> std::borrow::Cow<'_, str> {
        if self.compacting.is_some() {
            "Summarising earlier turns".into()
        } else if self.pending_approval().is_some() {
            "Waiting for your approval".into()
        } else if let Some((retry, deadline)) = self.retry.as_ref().filter(|_| self.status.active())
        {
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
        if self.pending_approval().is_some() {
            Some("waiting for approval")
        } else if self.running_command {
            Some("running run_command")
        } else if self.retry.is_some() {
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
        if self.compacting.is_some() {
            return "compacting".into();
        }
        if self.pending_approval().is_some() {
            return "approve?".into();
        }
        if let Some((retry, _)) = self.retry.as_ref().filter(|_| self.status.active()) {
            return format!("retry {}/{}", retry.retry, retry.limit);
        }
        match &self.status {
            Status::Connecting | Status::Streaming if self.running_command => "running".into(),
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
        // A restart never replays queued text: it comes back as the draft (issue #338).
        snapshot.restore_queue_saved();
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
        if self.replace_pending() {
            // The first Up only releases the rejected text for editing.
            self.rejected = None;
            return;
        }
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
                let last = self.history.len() - 1;
                // Recalling the text already in the prompt would look like a no-op.
                if last > 0 && self.history[last] == self.draft.trim() {
                    last - 1
                } else {
                    last
                }
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

    /// A rejected command stays visible for correction and Up recall, but the next
    /// typed character replaces it. Cursor and editing keys keep it for editing.
    pub fn reject_draft(&mut self) {
        self.remember_submission();
        self.history_index = None;
        self.rejected = Some(self.draft.clone()).filter(|draft| !draft.trim().is_empty());
    }
    /// True while the prompt shows rejected text that the next character replaces.
    pub fn replace_pending(&self) -> bool {
        self.rejected.as_deref() == Some(self.draft.as_str())
    }
    /// Types one character; it replaces rejected text that is still pending.
    pub fn type_char(&mut self, ch: char) {
        if self.replace_pending() {
            self.clear_draft();
        }
        self.insert(&ch.to_string());
    }

    /// Ctrl+O: switch every tool result between one line and full text (issue #258).
    pub fn toggle_tool_results(&mut self) {
        self.tool_results_expanded = !self.tool_results_expanded;
    }

    pub fn clear_draft(&mut self) {
        self.rejected = None;
        self.history_index = None;
        self.unsent = None;
        self.draft.clear();
        self.cursor = 0;
    }
    pub fn home(&mut self) {
        self.rejected = None;
        self.cursor = 0;
    }
    pub fn end(&mut self) {
        self.rejected = None;
        self.cursor = self.draft.len();
    }
    pub fn left(&mut self) {
        self.rejected = None;
        self.cursor = self.draft[..self.cursor()]
            .grapheme_indices(true)
            .next_back()
            .map(|(index, _)| index)
            .unwrap_or(0);
    }
    pub fn right(&mut self) {
        self.rejected = None;
        let cursor = self.cursor();
        self.cursor = cursor
            + self.draft[cursor..]
                .graphemes(true)
                .next()
                .map(str::len)
                .unwrap_or(0);
    }
    pub fn backspace(&mut self) {
        self.rejected = None;
        let end = self.cursor();
        self.left();
        self.draft.replace_range(self.cursor..end, "");
    }
    pub fn delete(&mut self) {
        self.rejected = None;
        let start = self.cursor();
        self.right();
        self.draft.replace_range(start..self.cursor, "");
        self.cursor = start;
    }
    /// Moves to the start of the previous whitespace-delimited word (Ctrl+Left, Alt+B).
    pub fn word_left(&mut self) {
        self.rejected = None;
        self.cursor = self.draft[..self.cursor()]
            .trim_end()
            .char_indices()
            .rev()
            .find(|(_, ch)| ch.is_whitespace())
            .map(|(index, ch)| index + ch.len_utf8())
            .unwrap_or(0);
    }
    /// Moves to the end of the next whitespace-delimited word (Ctrl+Right, Alt+F).
    pub fn word_right(&mut self) {
        self.rejected = None;
        let cursor = self.cursor();
        let rest = &self.draft[cursor..];
        let word = rest.trim_start();
        let skipped = rest.len() - word.len();
        let length = word.find(char::is_whitespace).unwrap_or(word.len());
        self.cursor = cursor + skipped + length;
    }
    /// Deletes from the cursor to the end of the draft (Ctrl+K).
    pub fn kill_to_end(&mut self) {
        self.rejected = None;
        let cursor = self.cursor();
        self.draft.truncate(cursor);
        self.cursor = cursor;
    }
    pub fn delete_word(&mut self) {
        self.rejected = None;
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
        let cursor = self.cursor();
        let display = |text: &str| text.replace('\n', "↵");
        caret_window(
            &display(&self.draft[..cursor]),
            &display(&self.draft[cursor..]),
            width,
        )
    }

    /// Number of logical lines in the draft (at least one).
    pub fn draft_line_count(&self) -> usize {
        self.draft.bytes().filter(|byte| *byte == b'\n').count() + 1
    }

    /// At most `height` logical lines of the draft, one per row, scrolled so the
    /// cursor line is the last row once the draft is longer than the window. The
    /// cursor line carries the caret and follows it horizontally; other lines are
    /// cut to `width` cells. A single-line draft is exactly `draft_view`.
    pub fn draft_lines(&self, width: usize, height: usize) -> Vec<String> {
        if height == 0 {
            return Vec::new();
        }
        let cursor = self.cursor();
        let line = self.draft[..cursor]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count();
        let first = (line + 1).saturating_sub(height);
        self.draft
            .split('\n')
            .enumerate()
            .skip(first)
            .take(height)
            .map(|(number, text)| {
                let start = text.as_ptr() as usize - self.draft.as_ptr() as usize;
                if number == line {
                    caret_window(
                        &self.draft[start..cursor],
                        &self.draft[cursor..start + text.len()],
                        width,
                    )
                } else {
                    cut_cells(text, width)
                }
            })
            .collect()
    }

    /// Moves the cursor to the previous draft line, keeping its cell column.
    /// Returns false (nothing moved) on the first line, so Up can browse history.
    pub fn line_up(&mut self) -> bool {
        if self.replace_pending() {
            return false;
        }
        let cursor = self.cursor();
        let Some(newline) = self.draft[..cursor].rfind('\n') else {
            return false;
        };
        let column = self.draft[newline + 1..cursor].width();
        let start = self.draft[..newline]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        self.cursor = start + column_offset(&self.draft[start..newline], column);
        true
    }

    /// Moves the cursor to the next draft line, keeping its cell column.
    /// Returns false on the last line, so Down can browse history.
    pub fn line_down(&mut self) -> bool {
        if self.replace_pending() {
            return false;
        }
        let cursor = self.cursor();
        let Some(offset) = self.draft[cursor..].find('\n') else {
            return false;
        };
        let start = self.draft[..cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let column = self.draft[start..cursor].width();
        let next = cursor + offset + 1;
        let end = self.draft[next..]
            .find('\n')
            .map_or(self.draft.len(), |index| next + index);
        self.cursor = next + column_offset(&self.draft[next..end], column);
        true
    }

    pub fn begin(&mut self) -> Result<Vec<Message>, &'static str> {
        if self.status.active() {
            return Err(BUSY);
        }
        let prompt = self.draft.trim().to_string();
        if prompt.is_empty() {
            return Err("Enter a prompt");
        }
        if self.compacting.is_some() {
            return Err("Summarising earlier turns; wait for it, or press Esc to keep the draft");
        }
        let bytes: usize = self.messages.iter().map(|m| m.content.len()).sum::<usize>()
            + self.attachments.values().map(String::len).sum::<usize>()
            + self.tool_bytes()
            + self.compaction.as_ref().map_or(0, |c| c.summary.len());
        if bytes + prompt.len() > MAX_TEXT || self.messages.len() + 2 > MAX_MESSAGES {
            return Err(compaction::HARD_CAP);
        }
        self.remember_submission();
        if matches!(self.status, Status::Failed(_) | Status::Cancelled) {
            // Moving on abandons the interrupted turn: it stays visible but is
            // no longer retryable and is not part of the next request.
            self.interrupted.push(self.messages.len() - 2);
        }
        self.messages.push(Message {
            role: "user".into(),
            content: prompt,
        });
        self.clear_draft();
        Ok(self.start())
    }

    /// Enter while a turn runs (issue #338): queue the prompt instead of sending it. The
    /// prompt is cleared on success; on refusal (full queue, a command, an approval card
    /// waiting) the text stays in the prompt.
    pub fn queue_draft(&mut self) -> Result<(), &'static str> {
        // A card keeps its own keys (#264); queued text never answers it.
        if !self.status.active() || self.pending_approval().is_some() {
            return Err(BUSY);
        }
        let text = self.draft.trim().to_string();
        if text.is_empty() {
            return Err("Enter a prompt");
        }
        if text.starts_with('/') {
            return Err("Commands cannot be queued; press Enter again when the reply ends");
        }
        let held: usize = self.queue.iter().map(String::len).sum();
        if self.queue.len() >= MAX_QUEUED || held + text.len() > MAX_QUEUED_BYTES {
            return Err("Queue full (3 messages); your text stays in the prompt until one is sent");
        }
        self.remember_submission();
        self.queue.push(text);
        self.clear_draft();
        Ok(())
    }

    /// The messages waiting for the running turn to end, oldest first.
    pub fn queued_messages(&self) -> &[String] {
        &self.queue
    }

    /// After a completed turn: send the oldest queued message as the next turn. The text
    /// being typed stays in the prompt. `None` when nothing is queued or the turn did not
    /// complete; on a refusal the queue returns to the prompt instead.
    pub fn begin_queued(&mut self) -> Option<Result<Vec<Message>, &'static str>> {
        if self.queue.is_empty() || self.status != Status::Complete {
            return None;
        }
        let text = self.queue.remove(0);
        let (draft, cursor) = (std::mem::take(&mut self.draft), self.cursor);
        self.draft = text.clone();
        let result = self.begin();
        if result.is_ok() {
            self.draft = draft;
            self.cursor = cursor;
        } else {
            self.draft = draft;
            self.queue.insert(0, text);
            self.restore_queue();
        }
        Some(result)
    }

    /// [`Self::restore_queue`] for a saved copy, which has no history to fall back on:
    /// queued text always lands in the draft, and only the tail of an over-long typed
    /// draft is cut to fit [`MAX_DRAFT`].
    fn restore_queue_saved(&mut self) {
        let joined = self.queue.join("\n");
        let room = MAX_DRAFT.saturating_sub(joined.len() + 1);
        if !self.queue.is_empty() && self.draft.len() > room {
            let mut cut = room;
            while !self.draft.is_char_boundary(cut) {
                cut -= 1;
            }
            self.draft.truncate(cut);
        }
        self.restore_queue();
    }

    /// Return queued messages to the front of the prompt, one per line, never sent.
    pub fn restore_queue(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let queued: Vec<String> = std::mem::take(&mut self.queue);
        let joined = queued.join("\n");
        let draft = if self.draft.trim().is_empty() {
            joined
        } else {
            format!("{joined}\n{}", self.draft)
        };
        if draft.len() <= MAX_DRAFT {
            self.draft = draft;
            self.history_index = None;
            self.unsent = None;
            self.end();
        } else {
            // Too long to join the text being typed: keep each in the Up-arrow history.
            for text in queued {
                self.remember(text);
            }
        }
    }

    /// Whether the user message at `user_index` opened a turn that was cancelled
    /// or failed and then abandoned for a new prompt.
    pub fn interrupted_turn(&self, user_index: usize) -> bool {
        user_index.is_multiple_of(2) && self.interrupted.contains(&user_index)
    }

    /// Model context ending at the user message `user_index`: earlier interrupted
    /// turns are omitted, except that a turn which applied a change keeps its prompt
    /// and a compact note of what was applied (issue #271); `user_index` itself is
    /// kept even if it was interrupted.
    pub fn request_through(&self, user_index: usize) -> Vec<Message> {
        let end = (user_index + 1).min(self.messages.len());
        self.compaction
            .iter()
            .map(Compaction::message)
            .chain(
                self.messages[..end]
                    .iter()
                    .enumerate()
                    .filter_map(|(index, message)| {
                        let user = index - index % 2;
                        let dropped = user != user_index && self.interrupted_turn(user);
                        let note = if dropped {
                            Some(self.applied_note(user + 1)?)
                        } else {
                            None
                        };
                        if dropped && note.is_none() {
                            return None;
                        }
                        let mut message = message.clone();
                        if let Some(block) = self.attachments.get(&index) {
                            message.content.push_str(block);
                        }
                        Some(match note {
                            // The reply of a kept interrupted turn is only the note.
                            Some(note) if index % 2 == 1 => vec![Message {
                                role: message.role,
                                content: note,
                            }],
                            Some(_) => vec![message],
                            None => self
                                .tool_steps(index)
                                .iter()
                                .flat_map(ToolStep::messages)
                                .chain(std::iter::once(message))
                                .collect(),
                        })
                    })
                    .flatten(),
            )
            .collect()
    }

    /// What the interrupted reply at `assistant_index` already changed: one line per
    /// executed mutating call whose outcome was not an error (denied, failed and
    /// never-answered calls are left out). `None` when nothing was applied.
    pub(crate) fn applied_note(&self, assistant_index: usize) -> Option<String> {
        use crate::chat_tools::{Tier, TOOLS};
        let applied: Vec<String> = self
            .tool_steps(assistant_index)
            .iter()
            .flat_map(|step| step.calls.iter().zip(&step.results))
            .filter(|(call, result)| {
                (!result.starts_with("error:") || killed_run(call, result))
                    && TOOLS.iter().any(|tool| {
                        tool.name == call.name && matches!(tool.tier, Tier::Mutating | Tier::Push)
                    })
            })
            .map(|(call, result)| {
                if killed_run(call, result) {
                    let command: String = call
                        .arguments
                        .get("command")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .chars()
                        .map(|c| if c.is_control() { ' ' } else { c })
                        .take(160)
                        .collect();
                    return format!(
                        "- {} {command}: cancelled mid-run, effects may be partial",
                        call.name
                    );
                }
                let target = call
                    .arguments
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(160)
                    .collect::<String>();
                let report: String = result
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(160)
                    .collect();
                format!("- {} {target}: {report}", call.name)
            })
            .collect();
        (!applied.is_empty()).then(|| {
            format!(
                "[This turn was interrupted. Already applied before it stopped:\n{}]",
                applied.join("\n")
            )
        })
    }

    /// Show the project-notes arrival `line` in the transcript unless this content
    /// (`fingerprint`) was already announced in this conversation. Returns whether a
    /// row was added. Display only: the row is not a message and grants nothing.
    pub fn show_project_notes(&mut self, fingerprint: u64, line: String) -> bool {
        if self.notes_fingerprint == Some(fingerprint) {
            return false;
        }
        self.notes_fingerprint = Some(fingerprint);
        self.notes_lines.push(line);
        // A file edited many times in one conversation keeps only the latest rows.
        let extra = self.notes_lines.len().saturating_sub(4);
        self.notes_lines.drain(..extra);
        true
    }

    /// Replace the `@path` refusal rows with `rows`, one per refused mention of the
    /// prompt at message `user_message` (issue #391); they are drawn right after it.
    /// Display only: not a message, grants nothing.
    pub fn show_mention_refusals(&mut self, user_message: usize, rows: Vec<String>) {
        self.mention_refusals = (user_message, rows);
    }

    /// The `@path` refusal rows and the user message they follow.
    pub fn mention_refusals(&self) -> (usize, &[String]) {
        (self.mention_refusals.0, &self.mention_refusals.1)
    }

    /// Arrival lines to draw above the transcript.
    pub fn notes_lines(&self) -> &[String] {
        &self.notes_lines
    }

    /// The always-allow rule that approved call `ordinal` (counted across the reply's
    /// rounds) of assistant message `assistant_index`, if one did.
    pub fn rule_of(&self, assistant_index: usize, ordinal: usize) -> Option<u64> {
        self.rule_marks
            .get(&assistant_index)?
            .iter()
            .find(|(at, _)| *at == ordinal)
            .map(|(_, rule)| *rule)
    }

    /// Tool rounds of the reply at `assistant_index` (empty when none).
    pub fn tool_steps(&self, assistant_index: usize) -> &[ToolStep] {
        self.tool_steps
            .get(&assistant_index)
            .map_or(&[], Vec::as_slice)
    }

    pub(crate) fn tool_bytes(&self) -> usize {
        self.tool_steps
            .values()
            .flatten()
            .map(ToolStep::bytes)
            .sum()
    }

    /// Saved tool rounds need schema v19, belong to replies, and are well formed.
    pub(crate) fn valid_tool_steps(&self, schema: u32) -> bool {
        self.tool_steps.is_empty()
            || (schema >= 19
                && self.tool_steps.len() <= MAX_MESSAGES / 2
                && self.tool_steps.iter().all(|(index, steps)| {
                    index % 2 == 1
                        && *index < self.messages.len()
                        && !steps.is_empty()
                        && steps.len() <= 64
                        && steps.iter().all(ToolStep::valid)
                }))
    }

    /// Attach file context to the user message at `user_index` for this and later
    /// requests; the transcript text is unchanged.
    pub fn attach(&mut self, user_index: usize, block: String) {
        if !block.is_empty()
            && self
                .messages
                .get(user_index)
                .is_some_and(|m| m.role == "user")
        {
            self.attachments.insert(user_index, block);
        }
    }

    /// Whether the user message at `user_index` already carries attached files.
    pub fn has_attachment(&self, user_index: usize) -> bool {
        self.attachments.contains_key(&user_index)
    }

    pub(crate) fn valid_interrupted(&self) -> bool {
        self.interrupted.windows(2).all(|pair| pair[0] < pair[1])
            && self
                .interrupted
                .iter()
                .all(|index| index.is_multiple_of(2) && index + 2 < self.messages.len())
    }

    pub fn retry(&mut self) -> Result<Vec<Message>, &'static str> {
        if !matches!(self.status, Status::Failed(_) | Status::Cancelled) {
            return Err("Only interrupted turns can be retried");
        }
        // The incomplete assistant message stays visible until an explicit retry.
        self.messages.pop();
        self.sources.remove(&self.messages.len());
        self.tool_steps.remove(&self.messages.len());
        self.rule_marks.remove(&self.messages.len());
        Ok(self.start())
    }

    fn start(&mut self) -> Vec<Message> {
        let request = self.request_through(self.messages.len() - 1);
        self.messages.push(Message {
            role: "assistant".into(),
            content: String::new(),
        });
        self.attempt += 1;
        self.approval = None;
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

    /// Show `card` (appearing at `now`) while this attempt's turn waits for the
    /// owner. False (and not shown) for a stale attempt or a finished turn.
    /// Whether the card is on screen now. Arming ([`crate::chat_tools::approval::ARM`])
    /// starts when it first becomes visible and restarts each time it is shown again.
    pub fn approval_visible(&mut self, visible: bool, now: std::time::Instant) {
        if let Some((_, shown)) = &mut self.approval {
            match (visible, *shown) {
                (true, None) => *shown = Some(now),
                (false, _) => *shown = None,
                (true, Some(_)) => {}
            }
        }
    }

    /// Whether the last draw put the pending card's header and args rows in the
    /// viewport. Pass it as part of `visible` to [`Self::approval_visible`].
    pub fn approval_header_drawn(&self) -> bool {
        self.approval_header_drawn.get()
    }

    /// Called by the draw: the card's header rows are (not) in the viewport.
    pub fn set_approval_header_drawn(&self, drawn: bool) {
        self.approval_header_drawn.set(drawn);
    }

    /// Whether the last draw showed the whole card, so `y` and `a` may answer it.
    pub fn approval_whole(&self) -> bool {
        self.approval_whole.get()
    }

    /// Called by the draw: the whole card is (not) in the viewport.
    pub fn set_approval_whole(&self, whole: bool) {
        self.approval_whole.set(whole);
    }

    pub fn await_approval(
        &mut self,
        attempt: u64,
        card: crate::chat_tools::approval::Card,
        now: std::time::Instant,
    ) -> bool {
        if self.attempt != attempt || !self.status.active() {
            return false;
        }
        if let Some(timing) = &mut self.timing {
            timing.pause(now);
        }
        // A new card is unseen until a draw says otherwise (issue #303).
        self.approval_header_drawn.set(false);
        self.approval_whole.set(false);
        self.approval = Some((card, None));
        true
    }

    /// The pending action card, while the turn is still running.
    pub fn pending_approval(&self) -> Option<&crate::chat_tools::approval::Card> {
        self.approval
            .as_ref()
            .filter(|_| self.status.active())
            .map(|(card, _)| card)
    }

    /// Record a key press at `now`. Call before routing the key (issue #264).
    pub fn note_key(&mut self, now: std::time::Instant) {
        self.key_times = (self.key_times.1, Some(now));
    }

    /// Whether the current key (see [`Session::note_key`]) may answer a pending card:
    /// the draft is empty and no other key came in the previous
    /// [`crate::chat_tools::approval::ARM`]. Otherwise it is ordinary typing.
    pub fn approval_key_answers(&self, now: std::time::Instant) -> bool {
        use crate::chat_tools::approval::ARM;
        if self.pending_approval().is_none() || !self.draft.is_empty() {
            return false;
        }
        match self.key_times.0 {
            Some(previous) => now.saturating_duration_since(previous) >= ARM,
            None => true,
        }
    }

    /// The owner's key on the card: `y` approves, `n` denies, `a` always allows (not on
    /// a push card; see [`crate::chat_tools::allow`]), and the card closes.
    /// Keys before the card is armed ([`crate::chat_tools::approval::ARM`] after it
    /// appeared), with text in the draft, or right after another key decide nothing
    /// (issue #264), and neither do other keys.
    pub fn answer_approval(
        &mut self,
        key: char,
        now: std::time::Instant,
    ) -> Option<crate::chat_tools::approval::Decision> {
        use crate::chat_tools::approval::{Decision, ARM};
        if !self.approval_key_answers(now) {
            return None;
        }
        let (_, shown) = self.approval.as_ref().filter(|_| self.status.active())?;
        let shown = shown.as_ref()?;
        if now.saturating_duration_since(*shown) < ARM {
            return None;
        }
        // Approving needs the whole card on screen; denying never does (issue #303).
        if key != 'n' && !self.approval_whole.get() {
            return None;
        }
        let decision = match key {
            'y' => Decision::Approve,
            'n' => Decision::Deny,
            'a' if crate::chat_tools::allow::available(self.pending_approval()?) => {
                Decision::AlwaysAllow
            }
            _ => return None,
        };
        if let Some(timing) = &mut self.timing {
            timing.resume(now);
        }
        self.running_command = decision != Decision::Deny
            && self.pending_approval().is_some_and(|card| {
                matches!(
                    card.kind,
                    crate::chat_tools::approval::CardKind::Shell { .. }
                )
            });
        self.approval = None;
        Some(decision)
    }

    pub fn cancel(&mut self) {
        self.restore_queue();
        self.compacting = None;
        self.approval = None;
        self.approval_header_drawn.set(false);
        self.approval_whole.set(false);
        self.running_command = false;
        self.pending_rule = None;
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

    /// Home / Ctrl+Home in the chat: jump to the oldest row when the prompt is empty (or
    /// with Ctrl); otherwise move the prompt cursor as before (issue #393).
    pub fn home_key(&mut self, ctrl: bool) {
        if ctrl || self.draft.is_empty() {
            // The same setter PgUp uses, so the saved anchor bookkeeping applies.
            self.scroll_rows(-(i32::MAX / 2));
        } else {
            self.home();
        }
    }
    /// End / Ctrl+End: jump to the latest row and follow new output, or move the cursor.
    pub fn end_key(&mut self, ctrl: bool) {
        if ctrl || self.draft.is_empty() {
            self.scroll_rows(i32::MAX / 2);
        } else {
            self.end();
        }
    }
    pub fn scroll_rows(&self, rows: i32) {
        let mut reading = self.reading.get();
        reading.move_rows(rows);
        self.reading.set(reading);
    }
    /// How often cached transcript bodies were rebuilt or re-measured (for tests).
    pub fn transcript_cache_stats(&self) -> crate::transcript_cache::Stats {
        self.transcript_cache.stats()
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
    fn next_sequence_value(&self) -> u64 {
        self.task_receipts
            .iter()
            .map(|r| r.sequence)
            .chain(self.commands.iter().map(|c| c.sequence))
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }
    fn next_console_sequence(&mut self) -> u64 {
        if self.next_sequence_value() > MAX_CONSOLE_SEQUENCE {
            self.renumber_console_sequences();
        }
        self.next_sequence_value()
    }
    /// Evictions leave gaps, so the lifetime counter would otherwise reach the cap with
    /// few commands left. Renumber 1.. in the existing order (parents stay earlier,
    /// ids are intent hashes and unchanged) and move the reader's command anchor along.
    fn renumber_console_sequences(&mut self) {
        let mut old: Vec<u64> = self
            .task_receipts
            .iter()
            .map(|r| r.sequence)
            .chain(self.commands.iter().map(|c| c.sequence))
            .filter(|sequence| *sequence > 0)
            .collect();
        old.sort_unstable();
        let renumbered = |sequence: u64| old.binary_search(&sequence).map_or(0, |at| at as u64 + 1);
        for reference in &mut self.task_receipts {
            reference.sequence = renumbered(reference.sequence);
        }
        for command in &mut self.commands {
            command.sequence = renumbered(command.sequence);
        }
        self.remap_reading_commands(|sequence| Some(renumbered(sequence)));
    }
    /// Move or drop the reader's anchor on a command (`None` drops it).
    fn remap_reading_commands(&self, map: impl Fn(u64) -> Option<u64>) {
        let mut reading = self.reading.get();
        reading.remap_block(|key| match key {
            crate::reading::BlockKey::Command(sequence) => {
                map(sequence).map(crate::reading::BlockKey::Command)
            }
            other => Some(other),
        });
        self.reading.set(reading);
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
        // One arrival per workspace and mission: a relaunch supersedes the earlier
        // banner instead of piling up. The superseded entry is display-only history
        // and the new record carries the live journal correlation. Restored on
        // failure so a refused admission changes nothing.
        let target = request.choice.target();
        let mission = request.choice.mission.name().to_owned();
        let before = self.commands.clone();
        self.commands.retain(|command| {
            !matches!(&command.intent, crate::command_intent::Intent::SelectionArrival { request }
                if request.choice.target() == target && request.choice.mission.name() == mission)
        });
        let admitted = self.admit_arrival(request, outcome);
        if admitted.is_err() {
            self.commands = before;
        }
        admitted
    }
    fn admit_arrival(
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
    /// Admit a command; a refusal leaves commands, task receipts and the reader's
    /// anchor exactly as found, even when the lifetime numbers were renumbered for it.
    fn admit_command(
        &mut self,
        text: String,
        intent: crate::command_intent::Intent,
        reveal: bool,
        after_messages: Option<usize>,
        state: crate::console_command::CommandState,
    ) -> Result<String, String> {
        let saved = (self.next_sequence_value() > MAX_CONSOLE_SEQUENCE).then(|| {
            (
                self.commands.clone(),
                self.task_receipts.clone(),
                self.reading.get(),
            )
        });
        let admitted = self.admit_command_numbered(text, intent, reveal, after_messages, state);
        if let (Err(_), Some((commands, receipts, reading))) = (&admitted, saved) {
            self.commands = commands;
            self.task_receipts = receipts;
            self.reading.set(reading);
        }
        admitted
    }
    fn admit_command_numbered(
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
        let keys = retirement_keys(&command);
        if self.retired_commands.contains(&keys[0]) {
            return Err("Retry the original command entry".into());
        }
        if keys[1..]
            .iter()
            .any(|key| self.retired_commands.contains(key))
        {
            return Err("Command correlation was already used; submit a new command".into());
        }
        self.commands.push(command);
        // Everything but capacity is judged on the full history, before any eviction, so
        // a duplicate correlation is refused at the budget exactly as under it.
        if !self.valid_command_structure(17) {
            let refusal = self.structure_refusal();
            self.commands.pop();
            return Err(refusal.into());
        }
        // Only a full budget evicts, and only settled commands; a refusal drops nothing.
        if !self.commands_within_budget() && !self.evict_settled_commands() {
            self.commands.pop();
            return Err(CAPACITY.into());
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
        self.valid_command_structure(schema)
            && self.commands_within_budget()
            && self.retired_commands.len() <= RETIRED_COMMANDS
            && self.retired_commands.iter().all(|key| {
                !key.is_empty() && key.len() <= 512 && !key.chars().any(char::is_control)
            })
    }
    /// Why the history just admitted to (its last command) breaks a structure rule.
    fn structure_refusal(&self) -> &'static str {
        let Some((last, earlier)) = self.commands.split_last() else {
            return "Invalid command history";
        };
        let key_of = |command: &crate::console_command::ConsoleCommand| {
            retirement_keys(command).split_off(1)
        };
        let keys = key_of(last);
        if earlier
            .iter()
            .any(|other| key_of(other).iter().any(|k| keys.contains(k)))
        {
            "Command correlation is already in use; submit a new command"
        } else if matches!(
            last.intent,
            crate::command_intent::Intent::ArchitectDraft { .. }
                | crate::command_intent::Intent::DispatchRun { .. }
        ) {
            "Command parent is missing or not yet settled; nothing was recorded"
        } else {
            "Invalid command history; nothing was recorded"
        }
    }
    /// Every command rule except the size budget.
    fn valid_command_structure(&self, schema: u32) -> bool {
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
            || self
                .commands
                .iter()
                .any(|c| !c.valid(self.messages.len()) || c.sequence > MAX_CONSOLE_SEQUENCE)
            || self
                .task_receipts
                .iter()
                .any(|reference| reference.sequence > MAX_CONSOLE_SEQUENCE)
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
        true
    }
    /// The command-history budget alone: 512 commands and 512 KiB, counting terminal
    /// metadata once against an immutable normalized base.
    pub(crate) fn commands_within_budget(&self) -> bool {
        command_budget_use(&self.commands) <= COMMAND_BUDGET_BYTES
            && self.commands.len() <= COMMAND_BUDGET_COUNT
    }
    /// Set when the history is 80% full: the next commands will drop the oldest settled ones.
    pub fn command_history_warning(&self) -> Option<String> {
        (self.commands.len() * 5 >= COMMAND_BUDGET_COUNT * 4
            || command_budget_use(&self.commands) * 5 >= COMMAND_BUDGET_BYTES * 4)
            .then(|| "Command history 80% full; /clear starts a fresh one".into())
    }
    /// Drop the fewest oldest settled commands that bring the history within budget, and
    /// say whether that was possible. Live (Pending, Submitted), selection, wayfinder,
    /// generated-plan and still-needed parent commands stay, so a restored Unknown or
    /// Refused entry is the usual victim. The command just admitted (last) stays too.
    /// Sizes are measured once per command, then the victims are found in one scan, so the
    /// cost does not grow with the number evicted.
    fn evict_settled_commands(&mut self) -> bool {
        let sizes = command_sizes(&self.commands);
        let (mut left, mut sum) = (
            self.commands.len(),
            sizes.iter().fold(0usize, |a, b| a.saturating_add(*b)),
        );
        let latest_dispatch = self.commands.iter().rposition(|other| {
            matches!(
                other.state,
                crate::console_command::CommandState::Control {
                    outcome: crate::control_command::Outcome::DispatchChanged { .. }
                }
            )
        });
        let mut dropped = Vec::new();
        for (index, size) in sizes
            .iter()
            .enumerate()
            .take(self.commands.len().saturating_sub(1))
        {
            if left <= COMMAND_BUDGET_COUNT && budget_bytes(left, sum) <= COMMAND_BUDGET_BYTES {
                break;
            }
            if self.evictable(index, latest_dispatch) {
                dropped.push(index);
                left -= 1;
                sum = sum.saturating_sub(*size);
            }
        }
        if left > COMMAND_BUDGET_COUNT || budget_bytes(left, sum) > COMMAND_BUDGET_BYTES {
            return false;
        }
        let kept: Vec<_> = (0..self.commands.len())
            .filter(|index| dropped.binary_search(index).is_err())
            .map(|index| self.commands[index].clone())
            .collect();
        let sequences: Vec<u64> = dropped
            .iter()
            .map(|index| self.commands[*index].sequence)
            .collect();
        let before = std::mem::replace(&mut self.commands, kept);
        if !self.commands_within_budget() {
            self.commands = before;
            return false;
        }
        self.retired_commands.extend(
            dropped
                .iter()
                .flat_map(|index| retirement_keys(&before[*index])),
        );
        let excess = self.retired_commands.len().saturating_sub(RETIRED_COMMANDS);
        self.retired_commands.drain(..excess);
        // A reader anchored to an evicted command must not keep a dangling anchor.
        self.remap_reading_commands(|sequence| {
            (!sequences.contains(&sequence)).then_some(sequence)
        });
        true
    }
    /// Name the pending plan draft's origin request (or none once the draft is gone), so
    /// command-history eviction keeps that command in any state: the draft is saved with
    /// it, and a save without it is refused (issue #475).
    pub fn protect_plan_origin(&mut self, origin: Option<&crate::planner_command::Request>) {
        if self.plan_origin.0.as_ref() != origin {
            self.plan_origin = PlanOrigin(origin.cloned());
        }
    }
    fn evictable(&self, index: usize, latest_dispatch: Option<usize>) -> bool {
        use crate::{
            command_intent::Intent, console_command::CommandState, control_command::Outcome,
            planner_command::Outcome as Planned,
        };
        let command = &self.commands[index];
        let enabling = latest_dispatch == Some(index)
            && matches!(
                command.state,
                CommandState::Control {
                    outcome: Outcome::DispatchChanged { enabled: true }
                }
            );
        let named_by_child =
            self.commands
                .iter()
                .any(|child| match (&child.intent, &command.intent) {
                    (Intent::ArchitectDraft { request }, Intent::Task { request: source }) => {
                        &request.source == source
                    }
                    (Intent::DispatchRun { request }, Intent::Control { request: source }) => {
                        &request.source == source
                    }
                    _ => false,
                });
        !(matches!(
            command.state,
            CommandState::Pending
                | CommandState::Submitted
                | CommandState::Selection { .. }
                | CommandState::Planner {
                    outcome: Planned::Generated { .. }
                }
        ) || command.intent.selection_request().is_some()
            || matches!(command.intent, Intent::Wayfinder { .. })
            || enabling
            || named_by_child
            || self.plan_origin.0.is_some()
                && command.intent.planner_request() == self.plan_origin.0.as_ref())
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
        if reference.sequence > MAX_CONSOLE_SEQUENCE {
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
    /// Saving must never fail because of where the user is reading (issue #375): a
    /// scroll anchor the stored data cannot account for is view state, so it is dropped
    /// (the saved offset then reads from the bottom) instead of refusing the snapshot.
    pub(crate) fn drop_unbounded_reading(&mut self, schema: u32) {
        if !self.valid_reading(schema) {
            self.reading.set(crate::reading::Viewport::default());
        }
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
                .sum::<usize>()
                + self
                    .tool_steps
                    .values()
                    .flatten()
                    .map(|step| {
                        // Collapsed: call row + cut result per call. Expanded (Ctrl+O,
                        // #369): up to 200 result rows and a marker per call.
                        2 * step.calls.len()
                            + 1
                            + step
                                .results
                                .iter()
                                .map(|result| result.lines().count().min(200) + 1)
                                .sum::<usize>()
                    })
                    .sum::<usize>()
                // Compaction note (#187): marker, one row per summary line, blank row.
                + self.compaction().map_or(0, |note| {
                    note.summary.bytes().filter(|b| *b == b'\n').count() + 3
                })
                // Project notes (#322) render above the transcript.
                + self.notes_lines.len()
                + self.mention_refusals.1.len() + 1
                + 1
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
    /// Add one finished call to the last reply's tool steps. False (nothing added) when
    /// the step is invalid or would pass the conversation limit.
    fn push_tool_result(&mut self, call: ToolCall, result: String, new_round: bool) -> bool {
        let total =
            self.messages.iter().map(|m| m.content.len()).sum::<usize>() + self.tool_bytes();
        let step = ToolStep {
            calls: vec![call],
            results: vec![result],
        };
        if !step.valid() || total + step.bytes() > MAX_TEXT {
            return false;
        }
        // A chat proposal (#239) fills an empty prompt; typed text is kept.
        if let Some(command) = crate::chat_tools::propose::prefill(&step) {
            if self.draft.trim().is_empty() && self.history_index.is_none() {
                self.draft = command;
                self.end();
            }
        }
        let rounds = self.tool_steps.entry(self.messages.len() - 1).or_default();
        match rounds.last_mut() {
            Some(open) if !new_round && open.calls.len() < 64 => {
                open.calls.extend(step.calls);
                open.results.extend(step.results);
            }
            _ => rounds.push(step),
        }
        true
    }

    pub fn apply(&mut self, attempt: u64, update: Update) {
        // A command killed by Esc, /stop or quit is reported after the cancel, by the
        // thread that reaped it (issue #333): it still joins the cancelled reply.
        if let Update::ToolResult {
            call,
            result,
            new_round,
        } = &update
        {
            if self.attempt == attempt
                && self.status == Status::Cancelled
                && !self.messages.is_empty()
                && result.starts_with(crate::chat_tools::CANCELLED_RESULT)
            {
                self.push_tool_result(call.clone(), result.clone(), *new_round);
                return;
            }
        }
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
                | Update::ToolStep(_)
                | Update::ToolResult { .. }
                | Update::RuleRunning { .. }
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
        if !matches!(update, Update::Metrics(_) | Update::RuleRunning { .. }) {
            self.running_command = false;
        }
        // A rule only belongs to the call it announced: its result takes it, anything
        // else (Done, Failed, a new round) drops it.
        if !matches!(
            update,
            Update::Metrics(_) | Update::RuleRunning { .. } | Update::ToolResult { .. }
        ) {
            self.pending_rule = None;
        }
        match update {
            Update::RuleRunning { tool, rule } => {
                self.running_command = tool == "run_command";
                self.pending_rule = Some(rule);
            }
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
                let total = self.messages.iter().map(|m| m.content.len()).sum::<usize>()
                    + self.tool_bytes();
                if total + text.len() > MAX_TEXT {
                    self.status = Status::Failed(OUTPUT_LIMIT.into());
                } else {
                    self.messages.last_mut().unwrap().content.push_str(&text);
                    self.status = Status::Streaming;
                }
            }
            // Provider-level calls are executed by the tool loop, which reports a step.
            Update::ToolCalls(_) => {}
            Update::ToolStep(step) => {
                let total = self.messages.iter().map(|m| m.content.len()).sum::<usize>()
                    + self.tool_bytes();
                if !step.valid() || total + step.bytes() > MAX_TEXT {
                    self.status = Status::Failed(OUTPUT_LIMIT.into());
                } else {
                    // A chat proposal (#239) fills an empty prompt; typed text is kept.
                    if let Some(command) = crate::chat_tools::propose::prefill(&step) {
                        if self.draft.trim().is_empty() && self.history_index.is_none() {
                            self.draft = command;
                            self.end();
                        }
                    }
                    self.tool_steps
                        .entry(self.messages.len() - 1)
                        .or_default()
                        .push(step);
                }
            }
            Update::Stopped(_) | Update::Notice(_) => {}
            Update::ToolResult {
                call,
                result,
                new_round,
            } => {
                let rule = self.pending_rule.take();
                let ordinal = self
                    .tool_steps
                    .get(&(self.messages.len() - 1))
                    .map_or(0, |rounds| rounds.iter().map(|s| s.calls.len()).sum());
                if !self.push_tool_result(call, result, new_round) {
                    self.status = Status::Failed(OUTPUT_LIMIT.into());
                } else if let Some(rule) = rule {
                    self.rule_marks
                        .entry(self.messages.len() - 1)
                        .or_default()
                        .push((ordinal, rule));
                }
            }
            // A reply with no text is no answer (issue #397): fail it so the owner is told
            // and `retry` or a new prompt drops it from the next request.
            Update::Done => {
                let empty = self
                    .messages
                    .last()
                    .is_none_or(|message| message.content.trim().is_empty());
                self.status = if empty {
                    Status::Failed(NO_TEXT.into())
                } else {
                    Status::Complete
                };
            }
            Update::Failed(reason) => self.status = Status::Failed(reason),
        }
        if !self.status.active() {
            if matches!(self.status, Status::Failed(_)) {
                self.restore_queue();
            }
            self.running_command = false;
            self.pending_rule = None;
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

/// Whether `result` is that of a `run_command` killed mid-run (issue #333).
fn killed_run(call: &ToolCall, result: &str) -> bool {
    call.name == "run_command" && result.starts_with(crate::chat_tools::CANCELLED_RESULT)
}

pub struct App {
    pub sessions: Vec<Session>,
    pub selected: usize,
    pub notice: String,
    /// Rows shown under the transcript until the next key (`/allow` lists each rule on
    /// its own row; issue #320). Not saved.
    pub transient_rows: Vec<String>,
    pub models: Vec<String>,
    pub models_visible: bool,
    pub models_pending: bool,
    /// True once discovery has succeeded at least once, so `models` is trustworthy.
    pub models_loaded: bool,
    /// `/model NAME` typed before the catalog was loaded; resolved when discovery returns.
    pub pending_model: Option<(usize, String)>,
    pub models_notice: String,
    pub models_scroll: u16,
    /// Catalog row the model list's arrow keys point at.
    pub models_cursor: usize,
    pub completion: Option<crate::commands::Completion>,
    /// Transient server health; inert until a workstation starts its monitor.
    pub health: crate::health::HealthView,
    /// Side pane focus, cursor, presentation settings and mission list (view state only).
    pub pane: crate::side_pane::PaneUi,
    /// Configured `--num-ctx`, for the context-fill indicator; None shows `default`.
    pub num_ctx: Option<u32>,
}

impl App {
    /// Put the command-history warning after the notice already shown (the `/go`
    /// acknowledgement) instead of replacing it, once per notice.
    pub fn append_history_warning(&mut self, session: usize) {
        if let Some(warning) = self.sessions[session].command_history_warning() {
            if !self.notice.contains(&warning) {
                self.notice = if self.notice.is_empty() {
                    warning
                } else {
                    format!("{} · {warning}", self.notice)
                };
            }
        }
    }
    /// A card was raised in chat `index`. Only the selected chat shows it, so a
    /// background chat gets a notice (issue #273).
    pub fn note_approval_wait(&mut self, index: usize) {
        if index != self.selected {
            self.notice = format!("Chat {} needs approval", index + 1);
        }
    }

    pub fn new(model: String) -> Self {
        Self {
            sessions: vec![Session::new(model)],
            selected: 0,
            notice: String::new(),
            transient_rows: Vec::new(),
            models: Vec::new(),
            models_visible: false,
            models_pending: false,
            models_loaded: false,
            pending_model: None,
            models_notice: String::new(),
            models_scroll: 0,
            models_cursor: 0,
            completion: None,
            health: Default::default(),
            pane: Default::default(),
            num_ctx: None,
        }
    }

    pub fn receive_models(&mut self, result: Result<Vec<String>, String>) {
        self.models_pending = false;
        let queued = self.pending_model.take();
        match result {
            Ok(models) => {
                self.models_loaded = true;
                self.models = models;
                let current = &self.sessions[self.selected].model;
                self.models_cursor = self
                    .models
                    .iter()
                    .position(|model| crate::models::same_model(model, current))
                    .unwrap_or(0);
                self.models_notice = if self.models.is_empty() {
                    "No installed models reported".into()
                } else {
                    "Installed models · ↑↓ Enter or /model NAME selects for this chat".into()
                };
                if let Some((index, name)) = queued {
                    self.resolve_queued_model(index, &name);
                }
            }
            Err(error) => {
                self.models_notice =
                    format!("{error}; previous catalog retained · /models retries");
                if let Some((_, name)) = queued {
                    self.notice = format!(
                        "Cannot check installed models for {name}: {error}; /model NAME retries"
                    );
                }
            }
        }
    }

    /// Whether `/model NAME` must wait for the first discovery. When it must,
    /// the name is held and the caller starts discovery if none is running.
    pub fn queue_model(&mut self, name: &str) -> bool {
        if self.models_loaded {
            return false;
        }
        self.pending_model = Some((self.selected, name.into()));
        self.notice = "Checking installed models…".into();
        true
    }

    fn resolve_queued_model(&mut self, index: usize, name: &str) {
        if !crate::models::contains_model(&self.models, name) {
            let shown: Vec<&str> = self.models.iter().take(4).map(String::as_str).collect();
            self.notice = if shown.is_empty() {
                format!("Model {name} is not installed; no installed models reported · /models")
            } else if self.models.len() > shown.len() {
                format!(
                    "Model {name} is not installed; installed: {} (+{} more) · /models lists all",
                    shown.join(", "),
                    self.models.len() - shown.len()
                )
            } else {
                format!(
                    "Model {name} is not installed; installed: {} · /models",
                    shown.join(", ")
                )
            };
        } else if let Err(error) = self.select_model_for(index, name) {
            self.notice = format!("{error} (/model {name} was not applied)");
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
        self.select_model_for(self.selected, name)
    }

    fn select_model_for(&mut self, index: usize, name: &str) -> Result<(), &'static str> {
        if self.models_pending {
            return Err("Wait for model discovery to finish");
        }
        if !self
            .models
            .iter()
            .any(|model| crate::models::same_model(model, name))
        {
            return Err("Model not in the catalog; /models refreshes installed models");
        }
        let Some(session) = self.sessions.get_mut(index) else {
            return Err("The conversation is gone");
        };
        if session.status.active() {
            return Err("An active turn retains its model; cancel or wait for it first");
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
            if let Update::Notice(text) = &event.update {
                if session.attempt == event.attempt && session.status.active() {
                    self.notice = text.clone();
                }
                return;
            }
            session.apply(event.attempt, event.update);
        }
    }
}

/// Viewport of one line around a caret, in terminal cells: keeps the text
/// before the caret up to its share of `width` and as much after as fits.
fn caret_window(before: &str, after: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let right_budget = after.width().min(width / 3);
    let left_budget = width.saturating_sub(1 + right_budget);
    let mut used = 0;
    let mut skip = before.len();
    for (index, grapheme) in before.grapheme_indices(true).rev() {
        if used + grapheme.width() > left_budget {
            break;
        }
        used += grapheme.width();
        skip = index;
    }
    let mut visible = String::with_capacity(width + 4);
    visible.push_str(&before[skip..]);
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

/// The leading part of `text` that fits in `width` cells.
fn cut_cells(text: &str, width: usize) -> String {
    let mut used = 0;
    let mut end = 0;
    for (index, grapheme) in text.grapheme_indices(true) {
        if used + grapheme.width() > width {
            break;
        }
        used += grapheme.width();
        end = index + grapheme.len();
    }
    text[..end].to_string()
}

/// Byte offset in `line` of the first cluster boundary at or past `column` cells.
fn column_offset(line: &str, column: usize) -> usize {
    let mut used = 0;
    for (index, grapheme) in line.grapheme_indices(true) {
        if used >= column {
            return index;
        }
        used += grapheme.width();
    }
    line.len()
}
