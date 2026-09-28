//! Canonical native first-contact adapter. Only deterministic, receipt-backed
//! responses describe scope writes; model continuations remain discussion.
use crate::{
    model::Message,
    understanding::{Action, Brief, Flow, Mode, Request, Snapshot, Store},
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{runtime::Runtime, task::JoinHandle};
static IDS: AtomicU64 = AtomicU64::new(0);

/// Match the legacy first-contact vocabulary with word boundaries, without a model.
pub fn entry_mode(prompt: &str) -> Option<Mode> {
    let explicit = crate::commands::capability_prompt(prompt).ok().flatten();
    let prompt = explicit.unwrap_or(prompt);
    let lower = prompt.to_lowercase();
    let words: Vec<_> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|s| !s.is_empty())
        .collect();
    if words.windows(2).any(|w| {
        matches!(w[0], "wayfinder" | "wayfinding")
            && (matches!(w[1], "map" | "ticket" | "issue") || w[1].parse::<u64>().is_ok())
    }) {
        return Some(Mode::WorkThrough);
    }
    let mut first = words.as_slice();
    for prefix in [
        &["please"][..],
        &["could", "you"],
        &["can", "you"],
        &["would", "you"],
        &["i", "want", "to"],
        &["i", "need", "to"],
    ] {
        if first.starts_with(prefix) {
            first = &first[prefix.len()..];
            break;
        }
    }
    if first.first().is_some_and(|word| {
        matches!(
            *word,
            "explain"
                | "what"
                | "what's"
                | "why"
                | "how"
                | "status"
                | "review"
                | "diagnose"
                | "diagnosis"
                | "inspect"
                | "show"
        )
    }) {
        return explicit.map(|_| Mode::Chart);
    }
    let new = words.windows(2).any(|w| {
        w[0] == "new"
            && matches!(
                w[1],
                "project" | "app" | "application" | "service" | "repository" | "product"
            )
    });
    let consequential = words.iter().any(|word| {
        matches!(
            *word,
            "architecture" | "architectural" | "redesign" | "migrate" | "migration"
        )
    }) || words.windows(2).any(|w| {
        matches!(
            w,
            ["consequential", "change"] | ["cross", "cutting"] | ["platform", "wide"]
        )
    }) || words
        .windows(3)
        .any(|w| matches!(w, ["replace", "the", "system"]));
    (explicit.is_some() || new || consequential).then_some(Mode::Chart)
}

fn supplied_brief(prompt: &str) -> Option<Brief> {
    let mut fields = BTreeMap::new();
    for line in prompt.lines().filter(|line| !line.trim().is_empty()) {
        let (key, value) = line.split_once(':')?;
        let key = key.trim().to_lowercase();
        if !matches!(
            key.as_str(),
            "destination" | "scope" | "constraints" | "uncertainty"
        ) || fields.insert(key, value.trim().to_owned()).is_some()
        {
            return None;
        }
    }
    Some(Brief {
        destination: fields.remove("destination")?,
        scope: fields.remove("scope")?,
        constraints: fields.remove("constraints")?,
        uncertainty: fields.remove("uncertainty")?,
    })
}

pub struct Decision {
    pub state: Snapshot,
    /// None continues the model as discussion, with captured scope as reference.
    pub acknowledgment: Option<String>,
    pub receipt: Option<crate::model::ScopeReceiptRef>,
}
fn acknowledge(state: Snapshot, correlation: &str, detail: &str) -> Decision {
    let receipt = state
        .receipts
        .iter()
        .find(|receipt| receipt.request.correlation == correlation)
        .expect("acknowledgment follows its exact receipt");
    let actor = &receipt.actor;
    let recorded_revision = receipt.request.expected_revision + 1;
    let detail = if recorded_revision == state.revision {
        detail
    } else {
        "earlier scope receipt replayed · no new action taken"
    };
    let current = if state.confirmed {
        "confirmed"
    } else {
        "pending"
    };
    let guidance = if state.confirmed {
        "Scope agreement alone never approves or runs tasks.".into()
    } else if state.flow.is_some() && state.draft_revision == 1 {
        "Scope is not agreed yet. Provide four lines:\nDestination: …\nScope: …\nConstraints: …\nUncertainty: …\n/scope inspects the saved flow.".into()
    } else {
        let brief = state.brief.as_ref().expect("draft receipt has a brief");
        format!("Destination: {}\nScope: {}\nConstraints: {}\nUncertainty: {}\n\nReview /scope, then: confirm shared understanding {}", brief.destination, brief.scope, brief.constraints, brief.uncertainty, state.draft_revision)
    };
    let acknowledgment = format!(
        "Wayfinder · {detail}\nReceipt: {correlation} · {actor} · revision {recorded_revision}\nCurrent scope: {current} · revision {}\n\n{guidance}",
        state.revision
    );
    Decision {
        receipt: Some(crate::model::ScopeReceiptRef {
            correlation: correlation.into(),
            revision: recorded_revision,
        }),
        state,
        acknowledgment: Some(acknowledgment),
    }
}

/// Read-only routing result. Requests must pass the saved-intent barrier before dispatch.
pub enum Preparation {
    Request(Request),
    Discussion(Decision),
}

/// Verify a saved request against the exact user turn that caused it. No state reads
/// or mutations are needed, and prose never creates an action by itself.
pub fn request_matches_prompt(request: &Request, original_prompt: &str) -> bool {
    if crate::understanding::validate_request(request).is_err() {
        return false;
    }
    let Ok(capability) = crate::commands::capability_prompt(original_prompt) else {
        return false;
    };
    let prompt = capability.unwrap_or(original_prompt);
    match &request.action {
        Action::Enter { flow } => {
            flow.prompt == original_prompt
                && entry_mode(original_prompt) == Some(flow.mode)
                && supplied_brief(prompt).is_none()
                && !prompt
                    .trim()
                    .to_lowercase()
                    .starts_with("confirm shared understanding")
        }
        Action::Draft { brief } => supplied_brief(prompt).as_ref() == Some(brief),
        Action::Confirm { draft_revision } => {
            prompt
                .trim()
                .to_lowercase()
                .strip_prefix("confirm shared understanding")
                .and_then(|revision| revision.trim().parse::<u64>().ok())
                == Some(*draft_revision)
        }
    }
}

pub fn prepare(
    store: &Store,
    original_prompt: &str,
    correlation: &str,
    observed_revision: Option<u64>,
) -> Result<Preparation, String> {
    let prompt = crate::commands::capability_prompt(original_prompt)?.unwrap_or(original_prompt);
    let state = store.snapshot()?;
    if let Some(receipt) = state
        .receipts
        .iter()
        .find(|receipt| receipt.request.correlation == correlation)
    {
        if !request_matches_prompt(&receipt.request, original_prompt) {
            return Err("Understanding correlation already used for another prompt".into());
        }
        return Ok(Preparation::Request(receipt.request.clone()));
    }
    let confirmation = prompt.trim().to_lowercase();
    let request = if let Some(revision) = confirmation.strip_prefix("confirm shared understanding")
    {
        let revision: u64 = revision.trim().parse().map_err(|_| {
            "Use: confirm shared understanding DRAFT_REVISION, after reviewing /scope"
        })?;
        Some(Request {
            correlation: correlation.into(),
            expected_revision: revision,
            action: Action::Confirm {
                draft_revision: revision,
            },
        })
    } else if let Some(brief) = supplied_brief(prompt) {
        Some(Request {
            correlation: correlation.into(),
            expected_revision: observed_revision
                .ok_or("Open /scope before supplying a scope draft")?,
            action: Action::Draft { brief },
        })
    } else if state.brief.is_none() {
        entry_mode(original_prompt).map(|mode| Request {
            correlation: correlation.into(),
            expected_revision: 0,
            action: Action::Enter {
                flow: Flow {
                    mode,
                    prompt: original_prompt.into(),
                },
            },
        })
    } else {
        None
    };
    match request {
        Some(request) => {
            crate::understanding::validate_request(&request)?;
            Ok(Preparation::Request(request))
        }
        None => Ok(Preparation::Discussion(Decision {
            state,
            acknowledgment: None,
            receipt: None,
        })),
    }
}

/// Execute only an already admitted exact request. A racing first-contact loser
/// observes the winning flow and receives no receipt for its own unapplied operation.
pub fn dispatch(store: &Store, request: &Request) -> Result<Decision, String> {
    crate::understanding::validate_request(request)?;
    let (state, detail) = match &request.action {
        Action::Enter { .. } => {
            let (state, acknowledged) = store.enter_prepared(request)?;
            if !acknowledged {
                return Ok(Decision {
                    state,
                    acknowledgment: None,
                    receipt: None,
                });
            }
            (
                state,
                "flow entered · Shared Understanding pending · no task action taken",
            )
        }
        Action::Draft { .. } => (
            store.transact(request.clone())?,
            "scope draft saved for review · no task action taken",
        ),
        Action::Confirm { .. } => (
            store.transact(request.clone())?,
            "Shared Understanding confirmed · turn complete · no task action taken",
        ),
    };
    if !state
        .receipts
        .iter()
        .any(|receipt| receipt.request == *request)
    {
        return Err("Wayfinder request ended without its exact scope receipt".into());
    }
    Ok(acknowledge(state, &request.correlation, detail))
}

/// Convenience route for explicitly effect-authorized synchronous callers.
pub fn route(
    store: &Store,
    prompt: &str,
    correlation: &str,
    observed_revision: Option<u64>,
) -> Result<Decision, String> {
    match prepare(store, prompt, correlation, observed_revision)? {
        Preparation::Request(request) => dispatch(store, &request),
        Preparation::Discussion(decision) => Ok(decision),
    }
}

#[derive(Clone)]
pub struct Turn {
    pub session: usize,
    pub attempt: u64,
    pub model: String,
    pub messages: Vec<Message>,
}
pub struct Completion {
    pub turn: Turn,
    /// Exact operation identity is retained even when its write failed or lost entry admission.
    pub request: Option<Request>,
    pub result: Result<Decision, String>,
}
enum Stage {
    Preparing(JoinHandle<Result<Preparation, String>>),
    Ready(Request),
    Writing {
        request: Request,
        job: JoinHandle<Result<Decision, String>>,
    },
}
struct Pending {
    turn: Turn,
    stage: Stage,
}
pub struct Router {
    store: Store,
    pending: BTreeMap<usize, Pending>,
}
impl Router {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            pending: BTreeMap::new(),
        }
    }
    pub fn active(&self) -> bool {
        !self.pending.is_empty()
    }
    pub fn session_active(&self, session: usize) -> bool {
        self.pending.contains_key(&session)
    }
    pub fn request_pending(&self, request: &Request) -> bool {
        self.pending.values().any(|pending| match &pending.stage {
            Stage::Ready(active)
            | Stage::Writing {
                request: active, ..
            } => active == request,
            Stage::Preparing(_) => false,
        })
    }
    fn admit(&self, turn: &Turn) -> Result<(), String> {
        if turn.session >= crate::model::MAX_SESSIONS
            || self.pending.contains_key(&turn.session)
            || self.pending.len() >= crate::model::MAX_SESSIONS
        {
            return Err(
                "Wait for the previous scope routing receipt before retrying this conversation"
                    .into(),
            );
        }
        if turn
            .messages
            .last()
            .is_none_or(|message| message.role != "user")
        {
            return Err("Expected a user turn".into());
        }
        Ok(())
    }
    /// Only captures and reads state asynchronously; no scope mutation can occur here.
    pub fn start(
        &mut self,
        runtime: &Runtime,
        turn: Turn,
        revision: Option<u64>,
    ) -> Result<(), String> {
        self.admit(&turn)?;
        let prompt = turn.messages.last().unwrap().content.clone();
        let correlation = format!(
            "wayfinder-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            IDS.fetch_add(1, Ordering::Relaxed)
        );
        let store = self.store.clone();
        let job = runtime.spawn_blocking(move || prepare(&store, &prompt, &correlation, revision));
        self.pending.insert(
            turn.session,
            Pending {
                turn,
                stage: Stage::Preparing(job),
            },
        );
        Ok(())
    }
    pub fn prepared(&self) -> Vec<(Turn, Request)> {
        self.pending
            .values()
            .filter_map(|pending| match &pending.stage {
                Stage::Ready(request) => Some((pending.turn.clone(), request.clone())),
                _ => None,
            })
            .collect()
    }
    /// Small admission metadata for the UI loop; do not clone accumulated chat
    /// history while a ready operation waits for another command's save.
    pub fn prepared_origins(&self) -> Vec<(usize, u64, usize, Request)> {
        self.pending
            .values()
            .filter_map(|pending| match &pending.stage {
                Stage::Ready(request) => Some((
                    pending.turn.session,
                    pending.turn.attempt,
                    pending.turn.messages.len() - 1,
                    request.clone(),
                )),
                _ => None,
            })
            .collect()
    }

    /// Restore an explicitly retried operation to the inert queue. This never runs on restart alone.
    pub fn resume(&mut self, turn: Turn, request: Request) -> Result<(), String> {
        self.admit(&turn)?;
        crate::understanding::validate_request(&request)?;
        if !request_matches_prompt(&request, &turn.messages.last().unwrap().content) {
            return Err("Saved Wayfinder request does not match its user turn".into());
        }
        if self.request_pending(&request) {
            return Err("Exact Wayfinder request is already awaiting acknowledgment".into());
        }
        self.pending.insert(
            turn.session,
            Pending {
                turn,
                stage: Stage::Ready(request),
            },
        );
        Ok(())
    }
    pub fn dispatch_prepared(
        &mut self,
        runtime: &Runtime,
        session: usize,
        request: &Request,
    ) -> Result<(), String> {
        crate::understanding::validate_request(request)?;
        let pending = self
            .pending
            .get_mut(&session)
            .ok_or("Wayfinder prepared origin is unavailable")?;
        if !matches!(&pending.stage, Stage::Ready(active) if active == request) {
            return Err("Wayfinder prepared request changed or was already dispatched".into());
        }
        let store = self.store.clone();
        let exact = request.clone();
        let job = runtime.spawn_blocking(move || dispatch(&store, &exact));
        pending.stage = Stage::Writing {
            request: request.clone(),
            job,
        };
        Ok(())
    }
    pub fn withdraw_prepared(&mut self, session: usize, request: &Request) -> bool {
        if !self.pending.get(&session).is_some_and(
            |pending| matches!(&pending.stage, Stage::Ready(active) if active == request),
        ) {
            return false;
        }
        self.pending.remove(&session);
        true
    }
    pub fn poll(&mut self, runtime: &Runtime) -> Vec<Completion> {
        let finished: Vec<_> = self
            .pending
            .iter()
            .filter_map(|(id, pending)| {
                match &pending.stage {
                    Stage::Preparing(job) => job.is_finished(),
                    Stage::Writing { job, .. } => job.is_finished(),
                    Stage::Ready(_) => false,
                }
                .then_some(*id)
            })
            .collect();
        let mut completed = Vec::new();
        for id in finished {
            let pending = self.pending.remove(&id).unwrap();
            match pending.stage {
                Stage::Preparing(job) => match runtime.block_on(job).unwrap_or_else(|_| Err("Wayfinder preparation stopped; inspect /scope before retrying".into())) {
                    Ok(Preparation::Request(request)) => {
                        self.pending.insert(id, Pending { turn: pending.turn, stage: Stage::Ready(request) });
                    }
                    result => completed.push(Completion { turn: pending.turn, request: None, result: result.map(|prepared| match prepared {
                        Preparation::Discussion(decision) => decision,
                        Preparation::Request(_) => unreachable!(),
                    }) }),
                },
                Stage::Writing { request, job } => completed.push(Completion {
                    turn: pending.turn, request: Some(request),
                    result: runtime.block_on(job).unwrap_or_else(|_| Err("Wayfinder write stopped; inspect /scope before retrying the exact request".into())),
                }),
                Stage::Ready(_) => unreachable!(),
            }
        }
        completed
    }
}
