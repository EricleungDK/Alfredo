//! The capped call -> result -> continue loop of one chat turn.
use super::{definitions, dispatch, Tier, TOOLS};
use crate::{
    model::{Event, Message, ToolCall, ToolStep, Update},
    provider::Ollama,
};
use std::path::Path;
use tokio::sync::mpsc::{self, Sender};

/// Tool rounds one turn may run; a model asking for more is stopped with a notice.
pub const MAX_STEPS: usize = 8;
/// Calls run from one model reply; extra calls are answered with an error.
const MAX_CALLS: usize = 16;
/// Shown in the reply when the step cap stops the loop.
pub const STEP_CAP_NOTICE: &str = "Stopped: tool step limit reached";

/// Run one chat turn: request a reply with the tools offered; while the model asks for
/// tools, run them, report a [`Update::ToolStep`], and ask again with the results.
/// Emits the same events as [`Ollama::chat`] (one final `Done` or `Failed`).
pub async fn run_turn(
    provider: &Ollama,
    workspace: &Path,
    session: usize,
    attempt: u64,
    model: String,
    mut messages: Vec<Message>,
    sender: Sender<Event>,
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
        calls.truncate(MAX_CALLS);
        for call in &mut calls {
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
        let root = workspace.to_path_buf();
        let run = calls.clone();
        let results = tokio::task::spawn_blocking(move || {
            run.iter()
                .map(
                    |call| match TOOLS.iter().find(|tool| tool.name == call.name) {
                        Some(tool) if tool.tier != Tier::Read => {
                            format!(
                                "error: {} needs approval, which is not available",
                                call.name
                            )
                        }
                        _ => dispatch(&root, call),
                    },
                )
                .collect::<Vec<_>>()
        })
        .await;
        let Ok(results) = results else {
            send(Update::Failed("Tool execution stopped unexpectedly".into())).await;
            return;
        };
        let step = ToolStep { calls, results };
        messages.extend(step.messages());
        if !send(Update::ToolStep(step)).await {
            return;
        }
    }
}
