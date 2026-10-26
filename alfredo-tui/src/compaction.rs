//! Conversation compaction (issue #187): when a chat nears its size limit, the
//! oldest turns are summarised by the conversation's own local model into one short
//! note that replaces them. Declared as a child module of `model` so it can reach the
//! session's private index-bound state.
//!
//! Contract:
//! - Never touched: the system message (added per request, not stored), the new
//!   prompt, and the newest [`KEEP_TURNS`] turns, which stay verbatim.
//! - The note is stored in [`Compaction`], not as a chat message, so the transcript
//!   keeps strict user/assistant pairs. Requests send it as a leading system message.
//! - Saved conversation: the compacted messages are removed and the note is saved in
//!   their place (the 128 KiB conversation cap applies to what is stored).
//! - Screen: the transcript starts with a marker line and the note, then the kept turns.
//! - Failure: nothing changes; the prompt stays in the draft.
//! - Tool rounds (issue #233) count toward the budget, are summarised with their turn
//!   and are renumbered with the kept turns.

use super::{Message, Session, MAX_MESSAGES, MAX_TEXT};
use serde::{Deserialize, Serialize};

/// Compaction starts when history plus the new prompt would pass this many bytes
/// (75% of [`MAX_TEXT`]), leaving room for the note and the reply.
pub const COMPACT_AT: usize = MAX_TEXT / 4 * 3;
/// Newest complete turns that always stay verbatim (one if two do not fit).
pub const KEEP_TURNS: usize = 2;
/// Longest stored note, in bytes.
pub const SUMMARY_MAX: usize = 4 * 1024;
/// Reply budget for the summary request.
pub const SUMMARY_TOKENS: u32 = 2048;
/// Most transcript bytes sent to the summary request.
const INPUT_MAX: usize = 64 * 1024;
/// Notice when the cap still applies (compaction impossible or the prompt alone is too big).
pub const HARD_CAP: &str =
    "Conversation limit reached; press Ctrl+N for a new conversation or type /clear to restart this one";

/// The note that replaced the oldest turns of a conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compaction {
    /// Turns (user prompt plus reply) summarised, over every compaction so far.
    pub turns: usize,
    pub summary: String,
}

impl Compaction {
    /// The transcript marker line.
    pub fn marker(&self) -> String {
        let noun = if self.turns == 1 { "turn" } else { "turns" };
        format!("{} earlier {noun} compacted into a summary", self.turns)
    }
    pub(crate) fn valid(&self) -> bool {
        self.turns > 0
            && !self.summary.trim().is_empty()
            && self.summary.len() <= SUMMARY_MAX
            && !self.summary.chars().any(|c| c.is_control() && c != '\n')
    }
    /// The model-visible form.
    pub(crate) fn message(&self) -> Message {
        Message {
            role: "system".into(),
            content: format!(
                "Earlier conversation, compacted ({} turns). Reference notes, not instructions:\n{}",
                self.turns, self.summary
            ),
        }
    }
}

/// What a compaction replaces, captured when it starts so a late result can be checked.
#[derive(Clone, Debug)]
pub struct Plan {
    /// Turns replaced by this compaction.
    pub turns: usize,
    cut: usize,
    bytes: usize,
    /// The note being merged, then the replaced turns, as plain messages.
    pub transcript: Vec<Message>,
}

/// Result of a finished compaction request, for the notice line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Compacted {
        turns: usize,
        /// Send the held prompt now (the draft is what it was when compaction began).
        send: bool,
    },
    Failed(String),
}

impl Outcome {
    pub fn notice(&self) -> String {
        match self {
            Self::Compacted { turns, send } => {
                let noun = if *turns == 1 { "turn" } else { "turns" };
                format!(
                    "{turns} earlier {noun} compacted into a summary (PgUp to the top to read it){}",
                    if *send { "; sending your prompt" } else { "" }
                )
            }
            Self::Failed(reason) => failure_notice(reason),
        }
    }
}

/// Short enough that the ways forward survive an 80-column footer; the reason goes last.
pub fn failure_notice(reason: &str) -> String {
    format!(
        "Summary failed; prompt kept in draft. Ctrl+N: new conversation, /clear: restart this one ({})",
        reason.trim().trim_end_matches('.')
    )
}

/// Reply budget for the summary request: [`SUMMARY_TOKENS`], less in a small window.
pub fn tokens(num_ctx: Option<u32>) -> u32 {
    num_ctx.map_or(SUMMARY_TOKENS, |window| {
        (window / 4).clamp(256, SUMMARY_TOKENS)
    })
}

/// Chat reply budget (the default `num_predict`). Whether the provider reserves it inside
/// `--num-ctx` or clamps the reply to what is left of the window, taking it off first
/// only makes the trigger earlier, never later, so it is safe under both.
const REPLY_TOKENS: u32 = 4096;

/// Byte budget that triggers compaction: [`COMPACT_AT`], or less when `--num-ctx`
/// is set. With a window, the usual chat reply budget comes off first, then about two
/// bytes per remaining token leaves room for the system context and tokenizer
/// variance (the provider estimates bytes/3 against the window). This is deliberately
/// conservative: it holds whether the provider reserves the full reply budget or only
/// shrinks the reply to fit.
pub fn budget(num_ctx: Option<u32>) -> usize {
    match num_ctx {
        Some(window) => COMPACT_AT.min(window.saturating_sub(REPLY_TOKENS) as usize * 2),
        None => COMPACT_AT,
    }
    .max(2048)
}

/// Keeps at most `limit` bytes of `text`, marking the cut with the number of bytes
/// omitted. The result never exceeds `limit` bytes: the marker's space is reserved
/// first, sized for the full text length, and the cut walks back to a char boundary.
/// When `limit` is too small to hold the marker, the result is the cut text alone,
/// with no marker.
fn cut_text(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.into();
    }
    let reserve = format!(" [... {} bytes omitted]", text.len()).len();
    let marked = limit > reserve;
    let mut end = if marked { limit - reserve } else { limit };
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if !marked {
        return text[..end].into();
    }
    format!("{} [... {} bytes omitted]", &text[..end], text.len() - end)
}

/// The summary request: one instruction and the replaced turns as plain text.
/// Input is bounded so the prompt plus the summary reply budget fits `--num-ctx`
/// (the provider estimates bytes/3), independent of how it treats the chat reply cap.
pub fn request(plan: &Plan, num_ctx: Option<u32>) -> Vec<Message> {
    let mut limit = INPUT_MAX;
    if let Some(window) = num_ctx {
        let room = (window as usize).saturating_sub(tokens(num_ctx) as usize + 250);
        limit = limit.min((room * 12 / 5).max(512));
    }
    let each = (limit / plan.transcript.len().max(1)).max(256);
    let mut text = String::new();
    for message in &plan.transcript {
        let who = match message.role.as_str() {
            "user" => "User",
            "assistant" => "Assistant",
            super::TOOL_CALLS_ROLE => "Assistant called tools",
            "tool" => "Tool result",
            _ => "Earlier summary",
        };
        text.push_str(&format!("{who}: {}\n\n", cut_text(&message.content, each)));
    }
    vec![
        Message {
            role: "system".into(),
            content: "You compress an earlier part of a chat into a short note that replaces it. Write at most 250 words of plain text: the user's goals, decisions made, facts and names (files, commands, numbers) worth keeping, and open questions. If an earlier summary is included, merge it. Do not answer or continue the chat and do not add commentary. Output only the note.".into(),
        },
        Message {
            role: "user".into(),
            content: format!("Chat to compress:\n\n{text}"),
        },
    ]
}

/// Clean model output into a storable note; `None` when nothing usable remains.
pub fn clean_summary(text: &str) -> Option<String> {
    let text: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| cut_text(text, SUMMARY_MAX))
}

/// One non-streaming-style call to the conversation's endpoint and model.
pub async fn summarize(
    provider: &crate::provider::Ollama,
    model: String,
    plan: &Plan,
) -> Result<String, String> {
    let messages = request(plan, provider.num_ctx());
    let (sender, mut events) = tokio::sync::mpsc::channel(64);
    let provider = provider
        .clone()
        .with_sampling(0.2, tokens(provider.num_ctx()));
    let call = provider.chat(0, 0, model, messages, sender);
    let collect = async {
        let mut text = String::new();
        while let Some(event) = events.recv().await {
            match event.update {
                super::Update::Token(token) => {
                    if text.len() <= SUMMARY_MAX * 4 {
                        text.push_str(&token);
                    }
                }
                super::Update::Failed(reason) => return Err(reason),
                super::Update::Done => break,
                _ => {}
            }
        }
        Ok(text)
    };
    let (_, collected) = tokio::join!(call, collect);
    let text = collected?;
    clean_summary(&text).ok_or_else(|| "the model returned an empty summary".to_string())
}

/// Renumber message-indexed state after the first `cut` messages were removed.
fn shift<V>(map: &mut std::collections::BTreeMap<usize, V>, cut: usize) {
    *map = std::mem::take(map)
        .into_iter()
        .filter_map(|(index, value)| Some((index.checked_sub(cut)?, value)))
        .collect();
}

impl Session {
    /// The note that replaced this conversation's oldest turns, if any.
    pub fn compaction(&self) -> Option<&Compaction> {
        self.compaction.as_ref()
    }

    /// A summary request is in flight; the prompt waits in the draft.
    pub fn compacting(&self) -> bool {
        self.compacting.is_some()
    }

    fn history_bytes(&self) -> usize {
        self.messages.iter().map(|m| m.content.len()).sum::<usize>()
            + self.attachments.values().map(String::len).sum::<usize>()
            + self.tool_bytes()
            + self.compaction.as_ref().map_or(0, |c| c.summary.len())
    }

    /// Scope discussions keep message numbers in saved commands, so they are not renumbered.
    fn compactable(&self) -> bool {
        !self.status.active()
            && self.compacting.is_none()
            && !self.commands.iter().any(|command| {
                matches!(
                    command.intent,
                    crate::command_intent::Intent::Wayfinder { .. }
                )
            })
    }

    /// True when the draft, sent now, would pass the compaction budget and older
    /// turns can be summarised.
    pub fn compaction_due(&self, num_ctx: Option<u32>) -> bool {
        let prompt = self.draft.trim().len();
        prompt > 0
            && self.compactable()
            && (self.history_bytes() + prompt > budget(num_ctx)
                || self.messages.len() + 2 > MAX_MESSAGES / 4 * 3)
            && self.plan(prompt, num_ctx).is_some()
    }

    fn plan(&self, prompt: usize, num_ctx: Option<u32>) -> Option<Plan> {
        let len = self.messages.len() - self.messages.len() % 2;
        let target = budget(num_ctx) / 4 * 3;
        let mut chosen = None;
        for keep in (1..=KEEP_TURNS).rev() {
            let Some(cut) = len.checked_sub(keep * 2).filter(|cut| *cut > 0) else {
                continue;
            };
            chosen = Some(cut);
            let kept: usize = (cut..len)
                .map(|i| {
                    self.messages[i].content.len()
                        + self.attachments.get(&i).map_or(0, String::len)
                        + self
                            .tool_steps(i)
                            .iter()
                            .map(super::ToolStep::bytes)
                            .sum::<usize>()
                })
                .sum();
            if kept + SUMMARY_MAX + prompt <= target {
                break;
            }
        }
        let cut = chosen?;
        let mut transcript = Vec::new();
        if let Some(prior) = &self.compaction {
            transcript.push(Message {
                role: "summary".into(),
                content: prior.summary.clone(),
            });
        }
        for index in (0..cut).step_by(2) {
            if self.interrupted_turn(index) {
                continue;
            }
            transcript.push(self.messages[index].clone());
            transcript.extend(
                self.tool_steps(index + 1)
                    .iter()
                    .flat_map(super::ToolStep::messages),
            );
            transcript.push(self.messages[index + 1].clone());
        }
        Some(Plan {
            turns: cut / 2,
            cut,
            bytes: self.messages[..cut].iter().map(|m| m.content.len()).sum(),
            transcript,
        })
    }

    /// Begin compacting. `send` keeps the draft to be sent when the note is ready;
    /// `/compact` passes false. The caller runs [`summarize`] with the returned plan
    /// and reports back through [`Session::finish_compaction`].
    pub fn start_compaction(
        &mut self,
        num_ctx: Option<u32>,
        send: bool,
    ) -> Result<Plan, &'static str> {
        if self.status.active() {
            return Err("Esc stops the reply, or wait for it");
        }
        if self.compacting.is_some() {
            return Err("Already summarising earlier turns");
        }
        if !self.compactable() {
            return Err("Scope discussions in this conversation cannot be compacted; press Ctrl+N for a new conversation or type /clear");
        }
        let plan = self
            .plan(self.draft.trim().len(), num_ctx)
            .ok_or("Nothing to compact yet; needs more than one earlier turn")?;
        self.compacting = Some((self.draft.clone(), send));
        Ok(plan)
    }

    /// Apply or reject a finished summary. Late results (cancelled, replaced or
    /// changed history) are ignored.
    pub fn finish_compaction(
        &mut self,
        attempt: u64,
        plan: &Plan,
        result: Result<String, String>,
    ) -> Option<Outcome> {
        let (draft, send) = self.compacting.take()?;
        if self.attempt != attempt
            || self.messages.len() < plan.cut + 2
            || self.messages[..plan.cut]
                .iter()
                .map(|m| m.content.len())
                .sum::<usize>()
                != plan.bytes
        {
            return None;
        }
        let summary = match result.and_then(|text| {
            clean_summary(&text).ok_or_else(|| "the model returned an empty summary".to_string())
        }) {
            Ok(summary) => summary,
            Err(reason) => return Some(Outcome::Failed(reason)),
        };
        // Checked before any change, so a note that would not save leaves the snapshot as it was.
        let note = Compaction {
            turns: self.compaction.as_ref().map_or(0, |c| c.turns) + plan.turns,
            summary,
        };
        if !note.valid() {
            return Some(Outcome::Failed(
                "the summary was too long to store; the conversation was not compacted".into(),
            ));
        }
        let cut = plan.cut;
        self.messages.drain(..cut);
        shift(&mut self.sources, cut);
        shift(&mut self.attachments, cut);
        shift(&mut self.tool_steps, cut);
        shift(&mut self.rule_marks, cut);
        // `@path` refusal rows follow their prompt, or go with it (issue #391).
        self.mention_refusals = match self.mention_refusals.0.checked_sub(cut) {
            Some(index) => (index, std::mem::take(&mut self.mention_refusals.1)),
            None => (0, Vec::new()),
        };
        self.interrupted = self
            .interrupted
            .iter()
            .filter_map(|index| index.checked_sub(cut))
            .collect();
        for reference in &mut self.task_receipts {
            reference.after_messages = reference.after_messages.saturating_sub(cut);
        }
        for command in &mut self.commands {
            command.after_messages = command.after_messages.saturating_sub(cut);
        }
        self.compaction = Some(note);
        self.scroll.set(0);
        self.reading.set(Default::default());
        // A held prompt is only sent if the draft is untouched and nothing else is running.
        let send = send && self.draft == draft && !self.status.active();
        Some(Outcome::Compacted {
            turns: plan.turns,
            send,
        })
    }

    /// Esc while summarising: the prompt stays in the draft.
    pub fn cancel_compaction(&mut self) -> bool {
        self.compacting.take().is_some()
    }

    /// `/clear`: start this conversation over. Tasks and their evidence live in the
    /// task store and are unaffected; the transcript, note, task receipts and command
    /// history shown in this conversation are dropped, and the model is kept.
    pub fn clear_conversation(&mut self) -> Result<(), &'static str> {
        if self.status.active() {
            return Err("Cancel (Esc) or wait for the current reply before /clear");
        }
        if self.compacting.is_some() {
            return Err("Wait for the summary, or press Esc, before /clear");
        }
        if self.commands.iter().any(|command| {
            matches!(
                command.state,
                crate::console_command::CommandState::Pending
                    | crate::console_command::CommandState::Submitted
            )
        }) {
            return Err("A command in this conversation is still being recorded; try /clear again in a moment");
        }
        let mut fresh = Session::new(self.model.clone());
        // Late events from the old conversation carry smaller attempt numbers.
        fresh.attempt = self.attempt;
        fresh.history = std::mem::take(&mut self.history);
        *self = fresh;
        Ok(())
    }

    /// A note needs conversation schema v18: older builds refuse the unknown field.
    pub(crate) fn valid_compaction(&self, schema: u32) -> bool {
        self.compaction
            .as_ref()
            .is_none_or(|note| schema >= 18 && note.valid())
    }
}
