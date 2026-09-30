//! Local Agent conversation continuity; retained model text never grants permissions.
use crate::{
    model::Message,
    tasks::{Action, Snapshot, Task, TaskStatus, TaskStore},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
const MAX_BYTES: usize = 512 * 1024;
const MAX_FILE: usize = 4 * 1024 * 1024;
const MAX_MESSAGES: usize = 8;
const FILE: &str = "agent-conversation.json";
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub agent: String,
    pub model: String,
    pub continued_from: Option<String>,
    pub reason: String,
    pub transcript_sha256: Option<String>,
    /// Set when the retained answer is what streamed before the run was cut short.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut: Option<Cut>,
}
impl Record {
    pub fn validate(&self, run: &str, model: &str) -> Result<()> {
        let identity = |s: &str| {
            !s.is_empty()
                && s.len() <= 100
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        };
        if !identity(&self.agent)
            || self.model != model
            || self.reason.is_empty()
            || self.reason.len() > 200
            || self.reason.chars().any(char::is_control)
            || self
                .continued_from
                .as_ref()
                .is_some_and(|s| !identity(s) || s == run)
            || (self.continued_from.is_none() && self.agent != run)
            || self
                .transcript_sha256
                .as_ref()
                .is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("Invalid Local Agent conversation binding".into());
        }
        Ok(())
    }
    pub fn summary(&self) -> String {
        format!(
            "Local Agent {} · {} · {}",
            self.agent,
            if self.continued_from.is_some() {
                "continued conversation"
            } else {
                "fresh conversation"
            },
            self.reason
        )
    }
}
/// A run cut short (steered or cancelled) while its answer streamed. What was
/// received is retained as an explicitly partial exchange, never as a completed one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cut {
    /// Whole seconds from model admission to the cut, when known.
    pub elapsed_secs: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Transcript {
    schema_version: u32,
    run: String,
    agent: String,
    model: String,
    messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cut: Option<Cut>,
}
fn valid_messages(messages: &[Message], complete: bool) -> bool {
    !messages.is_empty()
        && messages.len() <= MAX_MESSAGES
        && messages.len() % 2 == usize::from(!complete)
        && messages
            .iter()
            .enumerate()
            .all(|(i, m)| m.role == if i % 2 == 0 { "user" } else { "assistant" })
        && messages.iter().map(|m| m.content.len()).sum::<usize>() <= MAX_BYTES
}

pub struct Prepared {
    prior: Option<(String, String)>,
    history: Vec<Message>,
    reason: String,
}
fn rejection_count(snapshot: &Snapshot, mut id: u64) -> usize {
    let mut count = 0;
    for _ in 0..snapshot.tasks.len() {
        let Some(task) = snapshot.tasks.iter().find(|task| task.id == id) else {
            break;
        };
        // Count terminal rejection decisions, not interim risk holds or repair requests.
        if task.status == TaskStatus::Rejected {
            let rejected = snapshot
                .receipts
                .iter()
                .rev()
                .find_map(|r| match &r.request.action {
                    Action::Decide { task, decision }
                    | Action::ReviewAndRepair { task, decision }
                    | Action::ReviewArchitecture { task, decision }
                        if *task == id =>
                    {
                        Some(decision.outcome == crate::assessment::Outcome::Rejected)
                    }
                    Action::Assess { task, assessment } if *task == id => Some(!assessment.accept),
                    Action::Review { task, accept } if *task == id => Some(!accept),
                    _ => None,
                })
                .unwrap_or(false);
            count += usize::from(rejected);
        }
        if snapshot
            .plan_for_task(id)
            .is_some_and(|p| p.architecture.is_some())
        {
            break;
        }
        match task.repair_of {
            Some(parent) => id = parent,
            None => break,
        }
    }
    count
}

pub fn prepare(store: &TaskStore, snapshot: &Snapshot, task: &Task) -> Result<Prepared> {
    let fresh = |reason: &str| Prepared {
        prior: None,
        history: vec![],
        reason: reason.into(),
    };
    if snapshot
        .plan_for_task(task.id)
        .is_some_and(|p| p.architecture.is_some())
    {
        return Ok(fresh("Architect revised task; fresh Local Agent"));
    }
    let Some(parent_id) = task.repair_of else {
        return Ok(fresh("New task"));
    };
    let parent = snapshot
        .tasks
        .iter()
        .find(|t| t.id == parent_id)
        .ok_or("Missing repair parent")?;
    let evidence: crate::worker::Evidence =
        serde_json::from_str(&store.evidence(parent_id)?).map_err(|_| "Invalid repair evidence")?;
    if rejection_count(snapshot, parent_id) >= 2 {
        return Ok(fresh("Second or later rejection; fresh Local Agent"));
    }
    if parent.model != task.model {
        return Ok(fresh("Worker model changed"));
    }
    // Replaying a repeated answer as history anchors the model to it again.
    if evidence.detail.starts_with(crate::worker::NO_CHANGE) {
        return Ok(fresh("Previous attempt made no change; fresh Local Agent"));
    }
    let Some(record) = evidence.agent else {
        return Ok(fresh("Prior run has no recorded agent conversation"));
    };
    record.validate(&evidence.run, &parent.model)?;
    let Some(digest) = &record.transcript_sha256 else {
        return Ok(fresh("Prior model exchange did not complete"));
    };
    // A cut answer is partial: never replayed as the agent's earlier turn.
    if record.cut.is_some() {
        return Ok(fresh(
            "Previous attempt was steered mid-answer; fresh Local Agent",
        ));
    }
    // The record's model equals the parent's, which equals this task's.
    let transcript = read_transcript(store, &evidence.run, &record, digest)?;
    Ok(Prepared {
        prior: Some((record.agent, evidence.run)),
        history: transcript.messages,
        reason: "Repair continues the retained Local Agent exchange".into(),
    })
}
/// Digest-verified retained conversation of `run`.
fn read_transcript(
    store: &TaskStore,
    run: &str,
    record: &Record,
    digest: &str,
) -> Result<Transcript> {
    let mut bytes = Vec::new();
    crate::tasks::regular_file(&store.run_directory(run)?.join(FILE), false, false)?
        .take(MAX_FILE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FILE || format!("{:x}", Sha256::digest(&bytes)) != digest {
        return Err(
            "Local Agent conversation size or digest mismatch; repair remains unstarted".into(),
        );
    }
    let transcript: Transcript =
        serde_json::from_slice(&bytes).map_err(|_| "Malformed Local Agent conversation")?;
    if transcript.schema_version != 1
        || transcript.run != run
        || transcript.agent != record.agent
        || transcript.model != record.model
        || transcript.cut != record.cut
        || !valid_messages(&transcript.messages, true)
    {
        return Err("Local Agent conversation binding or history is invalid".into());
    }
    Ok(transcript)
}

/// The verified final model answer of a prior run, when its exchange completed.
/// A cut (partial) answer is never returned.
pub fn retained_answer(store: &TaskStore, evidence: &crate::worker::Evidence) -> Option<String> {
    let record = evidence.agent.as_ref()?;
    if record.cut.is_some() {
        return None;
    }
    let digest = record.transcript_sha256.as_deref()?;
    let transcript = read_transcript(store, &evidence.run, record, digest).ok()?;
    transcript.messages.last().map(|m| m.content.clone())
}

/// The verified last request and answer of a run, and whether that answer was
/// cut short, for the agent view. The error is one line on why the conversation
/// is not shown.
pub fn retained_exchange(
    store: &TaskStore,
    evidence: &crate::worker::Evidence,
) -> Result<(String, String, Option<Cut>)> {
    let record = evidence
        .agent
        .as_ref()
        .ok_or("No retained conversation for this run")?;
    let digest = record
        .transcript_sha256
        .as_deref()
        .ok_or("The model exchange did not complete; no conversation retained")?;
    let transcript = read_transcript(store, &evidence.run, record, digest)
        .map_err(|error| format!("Retained conversation unavailable: {error}"))?;
    let mut messages = transcript.messages.iter().rev();
    let answer = messages
        .next()
        .map(|m| m.content.clone())
        .unwrap_or_default();
    let prompt = messages
        .next()
        .map(|m| m.content.clone())
        .unwrap_or_default();
    Ok((prompt, answer, transcript.cut))
}

impl Prepared {
    /// Whether the request continues a retained conversation.
    pub fn continues(&self) -> bool {
        self.prior.is_some()
    }

    pub fn request(
        mut self,
        run: &str,
        model: &str,
        prompt: String,
    ) -> Result<(Record, Vec<Message>)> {
        // Reserve the provider's 128-KiB answer limit before admitting history.
        if self.history.len() + 2 > MAX_MESSAGES
            || self.history.iter().map(|m| m.content.len()).sum::<usize>() + prompt.len()
                > MAX_BYTES - crate::model::MAX_TEXT
        {
            self.history.clear();
            self.prior = None;
            self.reason = "Conversation budget reached; fresh Local Agent".into();
        }
        self.history.push(Message {
            role: "user".into(),
            content: prompt,
        });
        if !valid_messages(&self.history, false)
            || self.history.iter().map(|m| m.content.len()).sum::<usize>()
                > MAX_BYTES - crate::model::MAX_TEXT
        {
            return Err("Worker prompt exceeds Local Agent conversation budget".into());
        }
        let (agent, continued_from) = self
            .prior
            .map_or_else(|| (run.into(), None), |(agent, run)| (agent, Some(run)));
        Ok((
            Record {
                agent,
                model: model.into(),
                continued_from,
                reason: self.reason,
                transcript_sha256: None,
                cut: None,
            },
            self.history,
        ))
    }
}

pub fn retain(
    directory: &Path,
    run: &str,
    record: &mut Record,
    messages: Vec<Message>,
    answer: String,
) -> Result<()> {
    write(directory, run, record, messages, answer, None)
}

/// Retain what streamed of a run cut short, marked as a cut exchange. Callers
/// retain only text actually received.
pub fn retain_cut(
    directory: &Path,
    run: &str,
    record: &mut Record,
    messages: Vec<Message>,
    answer: String,
    elapsed_secs: Option<u64>,
) -> Result<()> {
    write(
        directory,
        run,
        record,
        messages,
        answer,
        Some(Cut { elapsed_secs }),
    )
}

fn write(
    directory: &Path,
    run: &str,
    record: &mut Record,
    mut messages: Vec<Message>,
    answer: String,
    cut: Option<Cut>,
) -> Result<()> {
    record.validate(run, &record.model)?;
    messages.push(Message {
        role: "assistant".into(),
        content: answer,
    });
    if !valid_messages(&messages, true) {
        return Err("Completed Local Agent conversation exceeds limits".into());
    }
    let transcript = Transcript {
        schema_version: 1,
        run: run.into(),
        agent: record.agent.clone(),
        model: record.model.clone(),
        messages,
        cut,
    };
    let bytes = serde_json::to_vec(&transcript).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FILE {
        return Err("Serialized Local Agent conversation exceeds limits".into());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(FILE))
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    File::open(directory)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    record.transcript_sha256 = Some(format!("{:x}", Sha256::digest(&bytes)));
    record.cut = cut;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conversation_budget_rollover_is_explicit_and_preserves_current_prompt() {
        let old = (0..MAX_MESSAGES)
            .map(|i| Message {
                role: if i % 2 == 0 { "user" } else { "assistant" }.into(),
                content: "prior".into(),
            })
            .collect();
        let prepared = Prepared {
            prior: Some(("task-1-1".into(), "task-2-2".into())),
            history: old,
            reason: "Continue".into(),
        };
        let (record, messages) = prepared
            .request("task-3-3", "model", "Current approved policy".into())
            .unwrap();
        assert_eq!(record.agent, "task-3-3");
        assert!(record.continued_from.is_none());
        assert!(record.reason.contains("budget"));
        assert_eq!(
            messages,
            vec![Message {
                role: "user".into(),
                content: "Current approved policy".into()
            }]
        );
        let prepared = Prepared {
            prior: None,
            history: vec![],
            reason: "New task".into(),
        };
        assert!(prepared
            .request(
                "task-1-1",
                "model",
                "x".repeat(MAX_BYTES - crate::model::MAX_TEXT + 1)
            )
            .is_err());
    }
    #[test]
    fn conversation_rejects_wrong_roles_incomplete_pairs_and_invalid_bindings() {
        let mut messages = vec![
            Message {
                role: "user".into(),
                content: "prompt".into(),
            },
            Message {
                role: "assistant".into(),
                content: "answer".into(),
            },
        ];
        assert!(valid_messages(&messages, true));
        assert!(!valid_messages(&messages, false));
        messages[0].role = "system".into();
        assert!(!valid_messages(&messages, true));
        let mut record = Record {
            agent: "task-1-1".into(),
            model: "model".into(),
            continued_from: None,
            reason: "New task".into(),
            transcript_sha256: None,
            cut: None,
        };
        record.validate("task-1-1", "model").unwrap();
        assert!(record.validate("task-2-2", "model").is_err());
        assert!(record.validate("task-1-1", "other-model").is_err());
        record.transcript_sha256 = Some("not-a-digest".into());
        assert!(record.validate("task-1-1", "model").is_err());
    }
}
