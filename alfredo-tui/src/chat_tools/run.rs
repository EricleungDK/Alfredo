//! The capped call -> result -> continue loop of one chat turn.
use super::{
    approval::{self, Decision, Gate, Outcome},
    budget::{Budget, CancelOnDrop, ROUND_LIMIT, ROUND_LIMIT_NOTE},
    cap_to, definitions, dispatch_budgeted, normalize_arguments, Tier, MAX_OUTPUT, TOOLS,
};
use crate::{
    model::{Event, Message, Stop, ToolCall, ToolStep, Update, MAX_TEXT, OUTPUT_LIMIT},
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
/// tools, run them, report each call as an [`Update::ToolResult`] when it finishes, and ask again with the results.
/// Emits the same events as [`Ollama::chat`] (one final `Done` or `Failed`).
/// Mutating calls ask `gate` one at a time; without a gate they are refused.
#[allow(clippy::too_many_arguments)]
pub async fn run_turn(
    provider: &Ollama,
    workspace: &Path,
    session: usize,
    attempt: u64,
    model: String,
    messages: Vec<Message>,
    sender: Sender<Event>,
    gate: Option<Gate>,
) {
    run_turn_limited(
        provider,
        workspace,
        session,
        attempt,
        model,
        messages,
        sender,
        gate,
        ROUND_LIMIT,
    )
    .await;
}

/// [`run_turn`] with the per-round read-time limit as a parameter (tests use small ones).
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub async fn run_turn_limited(
    provider: &Ollama,
    workspace: &Path,
    session: usize,
    attempt: u64,
    model: String,
    mut messages: Vec<Message>,
    sender: Sender<Event>,
    gate: Option<Gate>,
    round_limit: std::time::Duration,
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
    // Set when this future is dropped (Esc, /stop, switch, quit): blocking read calls
    // see it and end instead of running on (issue #257).
    let cancel = CancelOnDrop::default();
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
                if !send(Update::Notice(format!(
                    "Model {name} has no tool support; chat is running without file tools"
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
            send(Update::Stopped(Stop::OutputLimit)).await;
            send(Update::Token(
                "\n\n[Stopped: the conversation is near its size limit; /compact or ask again]"
                    .into(),
            ))
            .await;
            send(Update::Done).await;
            return;
        }
        if steps == MAX_STEPS {
            send(Update::Stopped(Stop::StepLimit)).await;
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
        // Bytes of this round already reported to the session.
        let mut reported = 0;
        // Only time spent in read calls counts against the round limit: owner think
        // time on an approval card and mutating calls do not.
        let mut read_spent = std::time::Duration::ZERO;
        for (index, call) in calls.iter().enumerate() {
            let used: usize = results.iter().map(String::len).sum::<usize>() + calls_bytes;
            let room = Room::of(provider.num_ctx(), held_text, held_window, used);
            last_round |= room.tight;
            let tier = TOOLS
                .iter()
                .find(|tool| tool.name == call.name)
                .map(|tool| tool.tier);
            let wrote = matches!(tier, Some(Tier::Mutating | Tier::Push));
            let mut result = match tier {
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
                        let result = gated(
                            workspace,
                            gate,
                            session,
                            attempt,
                            (call, index == 0),
                            &sender,
                        )
                        .await;
                        cap_to(result, room.limit.max(MIN_RESULT))
                    }
                },
                _ if read_spent >= round_limit => ROUND_LIMIT_NOTE.to_string(),
                _ if room.limit < MIN_RESULT => {
                    last_round = true;
                    room.note.to_string()
                }
                _ => {
                    let budget = Budget::new(round_limit - read_spent, cancel.flag());
                    let started = std::time::Instant::now();
                    let result = blocking(workspace, call, room.limit, &budget).await;
                    read_spent += started.elapsed();
                    result
                }
            };
            // The session refuses a result past the conversation limit. Results were
            // budgeted above; if this one still would not fit, a read result becomes a
            // note (a write's short report is kept, so the model knows it happened)
            // and the model answers instead of the turn failing.
            let step_bytes = |result: &str| {
                serde_json::to_string(call).map_or(0, |text| text.len()) + result.len()
            };
            if held_text + reported + step_bytes(&result) > MAX_TEXT {
                if !wrote {
                    result = NEAR_LIMIT_NOTE.to_string();
                }
                last_round = true;
            }
            if held_text + reported + step_bytes(&result) > MAX_TEXT {
                send(Update::Stopped(Stop::OutputLimit)).await;
                send(Update::Failed(OUTPUT_LIMIT.into())).await;
                return;
            }
            reported += step_bytes(&result);
            // Reported as soon as it finishes: a turn cancelled or failed during a
            // later call still keeps the ones that already ran (issue #271).
            if !send(Update::ToolResult {
                call: call.clone(),
                result: result.clone(),
                new_round: index == 0,
            })
            .await
            {
                return;
            }
            results.push(result);
        }
        let step = ToolStep { calls, results };
        let proposal = super::propose::prefill(&step);
        // The step was already reported call by call (`Update::ToolResult`).
        messages.extend(step.messages());
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

/// Run one call off the async thread, its output cut to `limit` bytes, under the
/// round's time and cancel `budget`.
async fn blocking(workspace: &Path, call: &ToolCall, limit: usize, budget: &Budget) -> String {
    let root = workspace.to_path_buf();
    let call = call.clone();
    let budget = budget.clone();
    tokio::task::spawn_blocking(move || dispatch_budgeted(&root, &call, limit, &budget))
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
    (call, new_round): (&ToolCall, bool),
    events: &Sender<Event>,
) -> String {
    let root = workspace.to_path_buf();
    let preview = call.clone();
    let card = match tokio::task::spawn_blocking(move || approval::card(&root, &preview)).await {
        Ok(Ok(card)) => card,
        Ok(Err(refusal)) => return format!("error: {refusal}"),
        Err(_) => return "error: approval preview stopped unexpectedly".into(),
    };
    // An always-allow rule (#238) stands in for the keypress only: the preview above
    // still refuses, and `apply` still checks the fingerprint.
    if let Some(rules) = gate.rules.clone() {
        let checked = card.clone();
        let matched = tokio::task::spawn_blocking(move || rules.matching(&checked))
            .await
            .ok()
            .flatten();
        if let Some(rule) = matched {
            // Display only (issue #320): tells the session this call runs under a rule,
            // so its row says `rule #N` and a command shows `running run_command`.
            let _ = events
                .send(Event {
                    session,
                    attempt,
                    update: Update::RuleRunning {
                        tool: call.name.clone(),
                        rule: rule.id,
                    },
                })
                .await;
            let root = workspace.to_path_buf();
            let log = gate.log.clone();
            let stop_guard = super::shell::StopOnDrop::default();
            let stop = stop_guard.stop();
            let late = Late::new(&stop_guard, events, session, attempt, call, new_round);
            let result = tokio::task::spawn_blocking(move || {
                super::shell::scoped(&stop, || {
                    late.report(decided(
                        &root,
                        &card,
                        Decision::Approve,
                        log.as_ref(),
                        Some(rule.id),
                    ))
                })
            })
            .await
            .unwrap_or_else(|_| "error: tool execution stopped unexpectedly".into());
            stop_guard.finish();
            return result;
        }
    }
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
    let rules = gate.rules.clone();
    // Aborting this turn (Esc, /stop, switch, retry, quit) drops the guard, which
    // stops a running command; the blocking call then logs `cancelled` (issue #302).
    let stop_guard = super::shell::StopOnDrop::default();
    let stop = stop_guard.stop();
    let late = Late::new(&stop_guard, events, session, attempt, call, new_round);
    // No await between here and the spawn: once spawned, the call runs and logs even
    // if this turn is aborted.
    let result = tokio::task::spawn_blocking(move || {
        super::shell::scoped(&stop, || {
            // `a` adds the rule, then runs this call like `y`. A rule that cannot be
            // saved still approves this one call.
            let rule = match (decision, &rules) {
                (Decision::AlwaysAllow, Some(rules)) => rules.add_for(&card).ok().map(|r| r.id),
                _ => None,
            };
            late.report(decided(&root, &card, decision, log.as_ref(), rule))
        })
    })
    .await
    .unwrap_or_else(|_| "error: tool execution stopped unexpectedly".into());
    stop_guard.finish();
    result
}

/// Reports the result of a `run_command` killed mid-run when the turn that ran it was
/// aborted (Esc, /stop, quit) and so never reads it (issue #333): the same
/// [`Update::ToolResult`] a finished call sends, from the thread that reaped the
/// process. The session accepts it after the cancel; nothing is re-run.
struct Late {
    events: Sender<Event>,
    session: usize,
    attempt: u64,
    call: ToolCall,
    new_round: bool,
    abandoned: super::shell::Abandoned,
}

impl Late {
    fn new(
        guard: &super::shell::StopOnDrop,
        events: &Sender<Event>,
        session: usize,
        attempt: u64,
        call: &ToolCall,
        new_round: bool,
    ) -> Self {
        Self {
            events: events.clone(),
            session,
            attempt,
            call: call.clone(),
            new_round,
            abandoned: guard.abandoned(),
        }
    }

    /// `result` unchanged, sent on first if it is a killed run nobody will read.
    fn report(self, result: String) -> String {
        if result.starts_with(super::shell::CANCELLED_RESULT) && self.abandoned.get() {
            // `try_send`: never blocks quit; a full channel only loses the row.
            let _ = self.events.try_send(Event {
                session: self.session,
                attempt: self.attempt,
                update: Update::ToolResult {
                    call: self.call,
                    result: result.clone(),
                    new_round: self.new_round,
                },
            });
        }
        result
    }
}

/// Run (or deny) a decided card and log the outcome; `rule` approved it, if any.
fn decided(
    root: &Path,
    card: &approval::Card,
    decision: Decision,
    log: Option<&approval::Log>,
    rule: Option<u64>,
) -> String {
    let (result, outcome) = match decision {
        Decision::Deny => (approval::DENIED.to_string(), Outcome::Denied),
        Decision::Approve | Decision::AlwaysAllow => {
            let started = std::time::Instant::now();
            match approval::apply(root, card) {
                Ok(output) => (super::cap(output), Outcome::Applied),
                // A command stopped mid-run (issue #302) was not applied, but it may have
                // partly run (issue #333); one stopped before it started ran nothing.
                Err(reason)
                    if matches!(card.kind, approval::CardKind::Shell { .. })
                        && reason.starts_with("cancelled (") =>
                {
                    let (_, detail) = reason.split_once('\n').unwrap_or((&reason, ""));
                    let secs = started.elapsed().as_secs();
                    (
                        super::cap(format!(
                            "{}{secs}s · killed; it may have partly run\n{detail}",
                            super::shell::CANCELLED_RESULT
                        )),
                        Outcome::Cancelled,
                    )
                }
                Err(reason)
                    if matches!(card.kind, approval::CardKind::Shell { .. })
                        && reason.starts_with("cancelled") =>
                {
                    (super::cap(format!("error: {reason}")), Outcome::Cancelled)
                }
                Err(reason) => (
                    super::cap(format!("error: {reason}")),
                    Outcome::Failed(reason),
                ),
            }
        }
    };
    if let Some(log) = log {
        log.append_by(card, &outcome, rule);
    }
    result
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
            Ok(Decision::Approve | Decision::AlwaysAllow) => Outcome::Failed(
                "turn cancelled before the approved call ran; nothing was written".into(),
            ),
            Ok(Decision::Deny) => Outcome::Denied,
            // Unanswered: the terminal logs it as cancelled.
            Err(_) => return,
        };
        log.append(&self.card, &outcome);
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use crate::chat_tools::budget::Cancel;
    use std::time::Duration;

    fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            name: name.into(),
            arguments,
        }
    }

    #[tokio::test]
    async fn a_call_after_the_round_deadline_is_not_run() {
        let dir = std::env::temp_dir().join(format!("alfredo-round-limit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "hello\n").unwrap();
        let live = Budget::new(Duration::from_secs(30), Cancel::default());
        let call = call("read_file", serde_json::json!({"path": "a.txt"}));
        assert!(blocking(&dir, &call, 4096, &live).await.contains("hello"));
        let spent = Budget::new(Duration::ZERO, Cancel::default());
        assert_eq!(blocking(&dir, &call, 4096, &spent).await, ROUND_LIMIT_NOTE);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
