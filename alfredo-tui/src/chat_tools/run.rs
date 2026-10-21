//! The capped call -> result -> continue loop of one chat turn.
use super::{
    approval::{self, Decision, Gate, Outcome},
    definitions, dispatch, normalize_arguments, Tier, TOOLS,
};
use crate::{
    model::{Event, Message, ToolCall, ToolStep, Update, MAX_TEXT},
    provider::Ollama,
};
use std::path::Path;
use tokio::sync::{
    mpsc::{self, Sender},
    oneshot,
};

/// Tool rounds one turn may run; a model asking for more is stopped with a notice.
pub const MAX_STEPS: usize = 8;
/// Calls run from one model reply; extra calls are answered with an error.
const MAX_CALLS: usize = 16;
/// Calls kept from one reply; any beyond are dropped (a model this far off is looping).
const MAX_KEPT: usize = 64;
/// Shown in the reply when the step cap stops the loop.
pub const STEP_CAP_NOTICE: &str = "Stopped: tool step limit reached";

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
    let provider = provider.clone().with_tools(definitions());
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
    let mut steps = 0;
    loop {
        let (inner, mut events) = mpsc::channel(64);
        let call = provider.chat(session, attempt, model.clone(), messages.clone(), inner);
        let forward = async {
            let mut calls: Vec<ToolCall> = Vec::new();
            let mut done = false;
            while let Some(event) = events.recv().await {
                match event.update {
                    Update::ToolCalls(found) => calls.extend(found),
                    Update::Done => done = true,
                    update => {
                        if !send(update).await {
                            return None;
                        }
                    }
                }
            }
            Some((calls, done))
        };
        let (_, outcome) = tokio::join!(call, forward);
        // A closed terminal, or a failure the provider already reported.
        let Some((mut calls, true)) = outcome else {
            return;
        };
        if calls.is_empty() {
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
        let mut results = Vec::with_capacity(calls.len());
        for (index, call) in calls.iter().enumerate() {
            let tier = TOOLS
                .iter()
                .find(|tool| tool.name == call.name)
                .map(|tool| tool.tier);
            let result = match tier {
                _ if index >= MAX_CALLS => {
                    format!("error: not run; at most {MAX_CALLS} tool calls per reply, ask again")
                }
                Some(Tier::Mutating | Tier::Push) => match &gate {
                    None => format!(
                        "error: {} needs approval, which is not available here",
                        call.name
                    ),
                    Some(gate) => gated(workspace, gate, session, attempt, call).await,
                },
                _ => blocking(workspace, call).await,
            };
            results.push(result);
        }
        let step = ToolStep { calls, results };
        // The session refuses a step past the conversation limit; stop here too.
        let held: usize = messages.iter().map(|m| m.content.len()).sum();
        if held + step.bytes() > MAX_TEXT {
            send(Update::Failed("Conversation output limit reached".into())).await;
            return;
        }
        messages.extend(step.messages());
        if !send(Update::ToolStep(step)).await {
            return;
        }
    }
}

/// Run one call off the async thread.
async fn blocking(workspace: &Path, call: &ToolCall) -> String {
    let root = workspace.to_path_buf();
    let call = call.clone();
    tokio::task::spawn_blocking(move || dispatch(&root, &call))
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
