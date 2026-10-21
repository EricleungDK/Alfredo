//! The capped call -> result -> continue loop of one chat turn.
use super::{
    approval::{self, Decision, Gate, Outcome},
    cap_to, definitions, dispatch_within, normalize_arguments, Tier, MAX_OUTPUT, TOOLS,
};
use crate::{
    model::{Event, Message, ToolCall, ToolStep, Update, MAX_TEXT, OUTPUT_LIMIT},
    provider::{Ollama, Round},
};
use std::{
    collections::HashSet,
    path::Path,
    sync::{Mutex, OnceLock},
};
use tokio::sync::{mpsc::Sender, oneshot};

/// Tool rounds one turn may run; a model asking for more is stopped with a notice.
pub const MAX_STEPS: usize = 8;
/// Calls run from one model reply; extra calls are answered with an error.
const MAX_CALLS: usize = 16;
/// Calls kept from one reply; any beyond are dropped (a model this far off is looping).
const MAX_KEPT: usize = 64;
/// Smallest tool result worth showing; with less room the result is replaced by a note.
const MIN_RESULT: usize = 256;
/// Conversation bytes kept free for the model's answer when results are budgeted.
const ANSWER_RESERVE: usize = 4096;
/// Tokens a `--num-ctx` request must keep free for the reply (the provider's floor).
const REPLY_FLOOR_TOKENS: usize = crate::provider::NUM_PREDICT_FLOOR as usize;
/// Result text when the conversation has no room left for a tool result.
pub const NEAR_LIMIT_NOTE: &str = "error: not shown; the conversation is near its size limit, answer with what you have or ask for a narrower range";
/// Result text when the `--num-ctx` window has no room left for a tool result.
pub const WINDOW_FULL_NOTE: &str = "error: not shown; the context window (--num-ctx) is nearly full, answer with what you have or ask for a narrower range";
/// Shown in the reply when the step cap stops the loop.
pub const STEP_CAP_NOTICE: &str = "Stopped: tool step limit reached";

/// Models Ollama refused `tools` for this session (issue #254); later turns skip tools.
fn no_tools_models() -> &'static Mutex<HashSet<String>> {
    static MODELS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    MODELS.get_or_init(Mutex::default)
}

/// Run one chat turn: request a reply with the tools offered; while the model asks for
/// tools, run them, report a [`Update::ToolStep`], and ask again with the results.
/// Emits the same events as [`Ollama::chat`] (one final `Done` or `Failed`).
/// Mutating calls ask `gate` one at a time; without a gate they are refused.
#[allow(clippy::too_many_arguments)]
pub async fn run_turn(
    provider: &Ollama,
    workspace: &Path,
    session: usize,
    attempt: u64,
    model: String,
    mut messages: Vec<Message>,
    sender: Sender<Event>,
    gate: Option<Gate>,
) {
    let known = |model: &str| {
        no_tools_models()
            .lock()
            .is_ok_and(|models| models.contains(model))
    };
    let mut plain = known(&model);
    let mut provider = if plain {
        provider.clone()
    } else {
        provider.clone().with_tools(definitions())
    };
    if plain {
        strip_tool_prompt(&mut messages);
    }
    let send = |update| {
        let sender = sender.clone();
        async move {
            sender
                .send(Event {
                    session,
                    attempt,
                    update,
                })
                .await
                .is_ok()
        }
    };
    let mut tools_bytes = if plain {
        0
    } else {
        serde_json::to_string(&definitions()).map_or(0, |text| text.len())
    };
    // A window too small for the tool definitions plus the reply floor cannot run with
    // tools at all (they alone are about 3k tokens); this turn runs as plain chat.
    if let Some(window) = provider.num_ctx().filter(|_| !plain) {
        let held: usize = messages
            .iter()
            .map(|m| m.role.len() + m.content.len())
            .sum();
        let needs = |bytes: usize| bytes / 3 + REPLY_FLOOR_TOKENS > window as usize;
        if needs(held + tools_bytes) && !needs(held) {
            plain = true;
            tools_bytes = 0;
            provider = provider.with_tools(Vec::new());
            strip_tool_prompt(&mut messages);
            if !send(Update::Token(format!(
                "[--num-ctx {window} is too small for the file tools; this reply runs without them]\n\n"
            )))
            .await
            {
                return;
            }
        }
    }
    let mut steps = 0;
    // Set once a round's results were squeezed out: the next round is offered no tools
    // so the model answers with what it has.
    let mut last_round = false;
    loop {
        if last_round && !plain {
            provider = provider.with_tools(Vec::new());
            plain = true;
        }
        // Tokens go straight to the session sender; `Done` is withheld by the
        // provider when the round asked for tools. A model that refuses tools has
        // its failure held back so the round is retried once in plain chat.
        let mut calls = match provider
            .chat_round_probing(
                session,
                attempt,
                model.clone(),
                messages.clone(),
                sender.clone(),
                !plain,
            )
            .await
        {
            Round::Calls(calls) => calls,
            Round::Stopped => return,
            Round::Rejected => {
                // Retry this round once without tools; later turns skip them without probing.
                plain = true;
                provider = provider.with_tools(Vec::new());
                if let Ok(mut models) = no_tools_models().lock() {
                    models.insert(model.clone());
                }
                strip_tool_prompt(&mut messages);
                let name: String = model.chars().filter(|c| !c.is_control()).collect();
                if !send(Update::Token(format!(
                    "[Model {name} has no tool support; chat is running without file tools]\n\n"
                )))
                .await
                {
                    return;
                }
                continue;
            }
        };
        if calls.is_empty() {
            // The provider already sent `Done` on the same path as the tokens.
            return;
        }
        if last_round {
            send(Update::Token(
                "\n\n[Stopped: the conversation is near its size limit; /compact or ask again]"
                    .into(),
            ))
            .await;
            send(Update::Done).await;
            return;
        }
        if steps == MAX_STEPS {
            send(Update::Token(format!(
                "\n\n[{STEP_CAP_NOTICE} after {MAX_STEPS} rounds; ask again to continue]"
            )))
            .await;
            send(Update::Done).await;
            return;
        }
        steps += 1;
        calls.truncate(MAX_KEPT);
        for call in &mut calls {
            call.arguments = normalize_arguments(&call.arguments);
            call.name = call
                .name
                .chars()
                .filter(|c| !c.is_control())
                .take(64)
                .collect();
            if call.name.is_empty() {
                call.name = "unnamed".into();
            }
        }
        let proposing = calls.iter().take(MAX_CALLS).any(super::propose::valid);
        let mut results: Vec<String> = Vec::with_capacity(calls.len());
        let held_text: usize = messages.iter().map(|m| m.content.len()).sum();
        let held_window: usize =
            held_text + messages.iter().map(|m| m.role.len()).sum::<usize>() + tools_bytes;
        let calls_bytes = serde_json::to_string(&calls).map_or(0, |text| text.len());
        for (index, call) in calls.iter().enumerate() {
            let used: usize = results.iter().map(String::len).sum::<usize>() + calls_bytes;
            let room = Room::of(provider.num_ctx(), held_text, held_window, used);
            last_round |= room.tight;
            let tier = TOOLS
                .iter()
                .find(|tool| tool.name == call.name)
                .map(|tool| tool.tier);
            let result = match tier {
                _ if index >= MAX_CALLS => {
                    format!("error: not run; at most {MAX_CALLS} tool calls per reply, ask again")
                }
                // A proposal hands off: nothing else in its reply runs or asks.
                _ if proposing && tier != Some(Tier::Propose) => {
                    "error: not run: proposal ends the turn; owner decides via /plan".into()
                }
                Some(Tier::Mutating | Tier::Push) => match &gate {
                    None => format!(
                        "error: {} needs approval, which is not available here",
                        call.name
                    ),
                    // An approved write still runs; only its (short) report is capped.
                    Some(gate) => {
                        let result = gated(workspace, gate, session, attempt, call).await;
                        cap_to(result, room.limit.max(MIN_RESULT))
                    }
                },
                _ if room.limit < MIN_RESULT => {
                    last_round = true;
                    room.note.to_string()
                }
                _ => blocking(workspace, call, room.limit).await,
            };
            results.push(result);
        }
        let mut step = ToolStep { calls, results };
        let proposal = super::propose::prefill(&step);
        // The session refuses a step past the conversation limit. Results were budgeted
        // above; if the step still would not fit, replace the read results with a note
        // (the short reports of writes that already ran are kept, so the model knows they
        // happened) and let the model answer instead of failing the turn.
        let held: usize = messages.iter().map(|m| m.content.len()).sum();
        if held + step.bytes() > MAX_TEXT {
            for (call, result) in step.calls.iter().zip(step.results.iter_mut()) {
                let wrote = TOOLS.iter().any(|tool| {
                    tool.name == call.name && matches!(tool.tier, Tier::Mutating | Tier::Push)
                });
                if !wrote {
                    *result = NEAR_LIMIT_NOTE.to_string();
                }
            }
            last_round = true;
        }
        if held + step.bytes() > MAX_TEXT {
            send(Update::Failed(OUTPUT_LIMIT.into())).await;
            return;
        }
        messages.extend(step.messages());
        if !send(Update::ToolStep(step)).await {
            return;
        }
        // A proposal hands off to the owner: the turn ends with the command in the prompt.
        if let Some(command) = proposal {
            send(Update::Token(format!(
                "Proposed `{command}`. It is in your prompt: edit it and press Enter to start, or clear it to drop it."
            )))
            .await;
            send(Update::Done).await;
            return;
        }
    }
}

/// Drop the sentences promising file tools from the chat system prompt.
fn strip_tool_prompt(messages: &mut [Message]) {
    for message in messages.iter_mut().filter(|m| m.role == "system") {
        message.content = crate::chat_context::without_tools(&message.content);
    }
}

/// What one tool result may take: the smaller of the tool cap, the room left in the
/// `--num-ctx` window (half the free tokens, at bytes/3 each, so the next round still
/// fits) and the room left under the conversation limit.
struct Room {
    limit: usize,
    /// Note used in place of a result when `limit` is under [`MIN_RESULT`].
    note: &'static str,
    /// The conversation limit (not the tool cap) bounds this result.
    tight: bool,
}

impl Room {
    fn of(num_ctx: Option<u32>, held_text: usize, held_window: usize, used: usize) -> Self {
        let text = MAX_TEXT.saturating_sub(held_text + used + ANSWER_RESERVE);
        let window = num_ctx.map(|window| {
            let prompt = (held_window + used) / 3;
            (window as usize).saturating_sub(prompt + REPLY_FLOOR_TOKENS) * 3 / 2
        });
        let mut limit = MAX_OUTPUT.min(text);
        let mut note = NEAR_LIMIT_NOTE;
        if let Some(window) = window.filter(|window| *window < limit) {
            limit = window;
            note = WINDOW_FULL_NOTE;
        }
        Self {
            limit,
            note,
            tight: text < MAX_OUTPUT,
        }
    }
}

/// Run one call off the async thread, its output cut to `limit` bytes.
async fn blocking(workspace: &Path, call: &ToolCall, limit: usize) -> String {
    let root = workspace.to_path_buf();
    let call = call.clone();
    tokio::task::spawn_blocking(move || dispatch_within(&root, &call, limit))
        .await
        .unwrap_or_else(|_| "error: tool execution stopped unexpectedly".into())
}

/// Build the card, wait for the owner, run an approved call and log the outcome.
/// A refused preview never asks; a closed gate or dropped reply is a denial (the
/// terminal logs a dropped one as cancelled).
async fn gated(
    workspace: &Path,
    gate: &Gate,
    session: usize,
    attempt: u64,
    call: &ToolCall,
) -> String {
    let root = workspace.to_path_buf();
    let preview = call.clone();
    let card = match tokio::task::spawn_blocking(move || approval::card(&root, &preview)).await {
        Ok(Ok(card)) => card,
        Ok(Err(refusal)) => return format!("error: {refusal}"),
        Err(_) => return "error: approval preview stopped unexpectedly".into(),
    };
    let (reply, decision) = oneshot::channel();
    let request = approval::Request {
        session,
        attempt,
        card: card.clone(),
        reply,
        log: gate.log.clone(),
    };
    if let Err(unsent) = gate.sender.send(request).await {
        let request = unsent.0;
        tokio::task::spawn_blocking(move || request.cancel())
            .await
            .ok();
        return approval::DENIED.into();
    }
    // If the turn is aborted (Esc) after the owner answered but before the call is
    // handed to the blocking pool, the guard still logs exactly one outcome.
    let mut guard = Undecided {
        decision: Some(decision),
        card: card.clone(),
        log: gate.log.clone(),
    };
    let Some(Ok(decision)) = (match guard.decision.as_mut() {
        Some(receiver) => Some(receiver.await),
        None => None,
    }) else {
        guard.decision = None;
        return approval::DENIED.into();
    };
    guard.decision = None;
    let root = workspace.to_path_buf();
    let log = gate.log.clone();
    // No await between here and the spawn: once spawned, the call runs and logs even
    // if this turn is aborted.
    tokio::task::spawn_blocking(move || {
        let (result, outcome) = match decision {
            Decision::Deny => (approval::DENIED.to_string(), Outcome::Denied),
            Decision::Approve => match approval::apply(&root, &card) {
                Ok(output) => (super::cap(output), Outcome::Applied),
                Err(reason) => (format!("error: {reason}"), Outcome::Failed(reason)),
            },
        };
        if let Some(log) = log {
            log.append(&card, &outcome);
        }
        result
    })
    .await
    .unwrap_or_else(|_| "error: tool execution stopped unexpectedly".into())
}

/// Logs the outcome of an answered card whose turn was dropped before it ran.
struct Undecided {
    decision: Option<oneshot::Receiver<Decision>>,
    card: approval::Card,
    log: Option<approval::Log>,
}

impl Drop for Undecided {
    fn drop(&mut self) {
        let (Some(mut receiver), Some(log)) = (self.decision.take(), &self.log) else {
            return;
        };
        let outcome = match receiver.try_recv() {
            Ok(Decision::Approve) => Outcome::Failed(
                "turn cancelled before the approved call ran; nothing was written".into(),
            ),
            Ok(Decision::Deny) => Outcome::Denied,
            // Unanswered: the terminal logs it as cancelled.
            Err(_) => return,
        };
        log.append(&self.card, &outcome);
    }
}
