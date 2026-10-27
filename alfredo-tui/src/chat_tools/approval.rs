//! Approval gate (issue #235, ADR 0004): a mutating chat tool call pauses the loop,
//! the owner sees a card ([`Card`]) and decides ([`Decision`]); the outcome is
//! appended to a bounded chat-approval [`Log`] in the State Directory (F4 activity).
//! The log is separate from the task store, so it never uses task receipt capacity.
//!
//! Generic over tools: a card comes from the tool family's preview function
//! ([`card`]). Always-allow rules ([`super::allow`]) are checked before prompting. A preview error refuses the call without asking.
use crate::{model::ToolCall, tasks::TaskStore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

/// A card ignores y/n this long after it appears, so typeahead cannot decide it.
pub const ARM: Duration = Duration::from_millis(500);
/// Records kept after the log is compacted; it holds at most twice this.
pub const MAX_LOG: usize = 500;
/// Log file name in the mission's State Directory.
pub const LOG_FILE: &str = "chat-approvals.jsonl";

/// What the owner decided. `AlwaysAllow` approves and adds a rule ([`super::allow`]);
/// push can never take it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Approve,
    Deny,
    AlwaysAllow,
}

/// How a card is presented; later slices add visibly different kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardKind {
    /// Writes working-tree files.
    Edit,
    /// Changes local Git state: commit, switch branch, merge (issue #237).
    Git,
    /// Publishes commits to a remote; shown distinctly, never always-allowed.
    Push,
    /// Runs a shell command (#236): in the Sandbox, or on the host when `escalated`.
    /// An always-allow rule (#238) must never cover an escalated card unless the rule
    /// itself is for escalated runs.
    Shell { escalated: bool },
}

/// The pending-action card: tool, arguments, a summary and a diff preview.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub tool: String,
    pub arguments: Value,
    pub kind: CardKind,
    /// One line naming the effect, such as `edit README.md`.
    pub summary: String,
    /// Diff or effect preview shown on the card.
    pub preview: String,
    /// State of the target when previewed; an approved call refuses if it changed.
    pub fingerprint: String,
}

impl Card {
    /// True for a run that would leave the Sandbox (host, network on).
    pub fn escalated(&self) -> bool {
        matches!(self.kind, CardKind::Shell { escalated: true })
    }
}

/// What came of one card.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Approved and run.
    Applied,
    /// Approved, but the run failed (nothing or not all was written).
    Failed(String),
    /// Denied with n.
    Denied,
    /// Never decided: the turn was cancelled (Esc, retry, switch); nothing ran.
    Cancelled,
}

impl Outcome {
    pub fn label(&self) -> String {
        match self {
            Outcome::Applied => "approved · applied".into(),
            Outcome::Failed(reason) => format!("approved · failed: {reason}"),
            Outcome::Denied => "denied".into(),
            Outcome::Cancelled => "denied · turn cancelled".into(),
        }
    }
}

/// One line of the chat-approval log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub at_ms: u64,
    pub tool: String,
    pub target: String,
    pub outcome: Outcome,
    /// A host-escalated run_command (#236).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub escalated: bool,
    /// Approved by this always-allow rule (#238), not by a keypress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<u64>,
}

/// Bounded append-only JSON-lines log of chat approval outcomes.
#[derive(Clone, Debug, PartialEq)]
pub struct Log {
    path: PathBuf,
}

impl Log {
    /// The log in `directory`.
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join(LOG_FILE),
        }
    }

    /// The log beside the mission's task store.
    pub fn for_store(store: &TaskStore) -> Result<Self, String> {
        Ok(Self::new(&store.conversation_directory()?))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The State Directory the log (and the undo journal, issue #325) lives in.
    pub fn directory(&self) -> &Path {
        self.path.parent().unwrap_or_else(|| Path::new("."))
    }

    /// Append one outcome; past `2 * MAX_LOG` records the oldest are dropped. Best
    /// effort: a failed write never blocks chat or tasks.
    pub fn append(&self, card: &Card, outcome: &Outcome) {
        self.append_by(card, outcome, None)
    }

    /// [`Log::append`] for a call approved by always-allow `rule`.
    pub fn append_by(&self, card: &Card, outcome: &Outcome, rule: Option<u64>) {
        let record = Record {
            at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64),
            tool: card.tool.chars().take(64).collect(),
            target: card
                .summary
                .chars()
                .filter(|c| !c.is_control())
                .take(200)
                .collect(),
            outcome: match outcome {
                Outcome::Failed(reason) => Outcome::Failed(
                    reason
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(300)
                        .collect(),
                ),
                other => other.clone(),
            },
            escalated: card.escalated(),
            rule,
        };
        let Ok(line) = serde_json::to_string(&record) else {
            return;
        };
        // One writer at a time across threads and processes on this mission.
        let Ok(lock) = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.path.with_extension("lock"))
        else {
            return;
        };
        if lock.lock().is_err() {
            return;
        }
        let mut records = self.records();
        if records.len() >= 2 * MAX_LOG {
            records.drain(..records.len() + 1 - MAX_LOG);
            records.push(record);
            let text: String = records
                .iter()
                .filter_map(|r| serde_json::to_string(r).ok())
                .map(|l| l + "\n")
                .collect();
            static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let temporary = self.path.with_extension(format!(
                "jsonl.{}-{}.tmp",
                std::process::id(),
                SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            if std::fs::write(&temporary, text).is_ok()
                && std::fs::rename(&temporary, &self.path).is_err()
            {
                let _ = std::fs::remove_file(&temporary);
            }
            return;
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = file.write_all(format!("{line}\n").as_bytes());
        }
    }

    /// Records, oldest first. Malformed lines are skipped.
    pub fn records(&self) -> Vec<Record> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        text.lines()
            .filter(|line| line.len() <= 4096)
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }
}

/// One pending decision sent from the tool loop to the terminal. Dropping
/// `reply` counts as a denial; [`Request::cancel`] also logs it.
#[derive(Debug)]
pub struct Request {
    pub session: usize,
    pub attempt: u64,
    pub card: Card,
    pub reply: oneshot::Sender<Decision>,
    pub log: Option<Log>,
}

impl Request {
    /// True when approving would run on the host (see [`Card::escalated`]).
    pub fn escalated(&self) -> bool {
        self.card.escalated()
    }

    /// Drop the request without a decision and log it as cancelled.
    pub fn cancel(self) {
        if let Some(log) = &self.log {
            log.append(&self.card, &Outcome::Cancelled);
        }
    }
}

/// Where the tool loop asks for decisions, and where outcomes are logged.
#[derive(Clone, Debug)]
pub struct Gate {
    pub sender: mpsc::Sender<Request>,
    pub log: Option<Log>,
    /// Always-allow rules checked before asking; `None` always asks.
    pub rules: Option<super::allow::Rules>,
    /// Identity of the turn this gate serves, stamped by `run_turn` (issue #325).
    pub(crate) turn: String,
}

impl Gate {
    pub fn new(sender: mpsc::Sender<Request>, log: Option<Log>) -> Self {
        Self {
            sender,
            log,
            rules: None,
            turn: String::new(),
        }
    }

    /// Check (and let `a` add to) `rules` before asking.
    pub fn with_rules(self, rules: super::allow::Rules) -> Self {
        Self {
            rules: Some(rules),
            ..self
        }
    }
}

/// Tool result the model gets for a denied call. Nothing ran.
pub const DENIED: &str =
    "error: the owner denied this action; nothing was changed. Do not retry it unless the owner asks.";

/// Build the card for a mutating call, or the reason it is refused before asking.
pub fn card(root: &Path, call: &ToolCall) -> Result<Card, String> {
    let arguments = super::normalize_arguments(&call.arguments);
    if !arguments.is_object() {
        return Err("arguments must be a JSON object".into());
    }
    if let Some(shown) = super::shell::preview(root, &call.name, &arguments) {
        let shown = shown?;
        return Ok(Card {
            tool: call.name.clone(),
            arguments,
            kind: CardKind::Shell {
                escalated: shown.escalated,
            },
            summary: shown.summary,
            preview: shown.preview,
            fingerprint: shown.cwd,
        });
    }
    let (change, kind) = match super::edit::preview(root, &call.name, &arguments) {
        Some(change) => (change?, CardKind::Edit),
        None => super::git_write::preview(root, &call.name, &arguments)
            .ok_or_else(|| format!("{} has no approval preview", call.name))??,
    };
    Ok(Card {
        tool: call.name.clone(),
        arguments,
        kind,
        summary: change.summary,
        preview: change.diff,
        fingerprint: change.fingerprint,
    })
}

/// Run an approved card: refuse when the target changed since the preview,
/// otherwise dispatch. `Err` is the failure reason.
pub fn apply(root: &Path, card: &Card) -> Result<String, String> {
    if matches!(card.kind, CardKind::Shell { .. }) {
        if super::shell::cwd(root, &card.arguments)? != card.fingerprint {
            return Err(
                "the cwd changed since the preview; nothing ran. Re-propose the command".into(),
            );
        }
    } else {
        let now = super::edit::fingerprint(root, &card.tool, &card.arguments)
            .or_else(|| super::git_write::fingerprint(root, &card.tool, &card.arguments))
            .ok_or_else(|| format!("{} has no approval preview", card.tool))??;
        if now != card.fingerprint {
            return Err(match card.kind {
                CardKind::Edit => "the file changed since the preview; nothing was written. Read it again and re-propose the change",
                CardKind::Shell { .. } => unreachable!("shell cards are checked above"),
                CardKind::Git | CardKind::Push => "the repository changed since the preview; nothing was run. Check git_status and re-propose",
            }
            .into());
        }
    }
    let output = super::dispatch(
        root,
        &ToolCall {
            name: card.tool.clone(),
            arguments: card.arguments.clone(),
        },
    );
    match output.strip_prefix("error: ") {
        Some(reason) => Err(reason.to_string()),
        None => Ok(output),
    }
}
