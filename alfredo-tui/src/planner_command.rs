//! Saved planner operations are provenance for drafts, never task authority.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub correlation: String,
    pub operation: Operation,
}
/// A review-triggered draft operation. The source review is canonical task
/// authority; the generated draft remains non-authoritative until separately saved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectRequest {
    pub request: Request,
    pub source: crate::tasks::Request,
}
impl ArchitectRequest {
    pub fn validate(&self) -> Result<(), String> {
        self.request.validate()?;
        if !text(&self.source.correlation, 160)
            || self.source.correlation == self.request.correlation
            || self.source.expected_revision >= 4096
            || serde_json::to_vec(self)
                .map_err(|error| error.to_string())?
                .len()
                > 160 * 1024
        {
            return Err("Invalid automatic Architect source identity or bounds".into());
        }
        let Operation::Architect { origin, .. } = &self.request.operation else {
            return Err("Automatic Architect request requires an Architect operation".into());
        };
        let crate::tasks::Action::ReviewArchitecture { task, decision } = &self.source.action
        else {
            return Err("Automatic Architect source must be an architecture review".into());
        };
        decision.validate()?;
        if *task != origin.task
            || !(1..=256).contains(task)
            || self.source.expected_revision + 1 != origin.review_revision
            || decision.failure != Some(crate::assessment::FailureKind::Architecture)
            || !decision.proposes_repair()
        {
            return Err("Automatic Architect source does not match its review route".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Operation {
    Generate {
        prompt: String,
        model: String,
        revision: u64,
        base_sha256: Option<String>,
    },
    Revise {
        prompt: String,
        revision: u64,
        base_sha256: String,
    },
    Architect {
        origin: crate::architecture::Origin,
        revision: u64,
    },
    Cancel {
        generation: Option<String>,
        draft_sha256: Option<String>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Outcome {
    Generated { draft_sha256: String, tasks: usize },
    Stopped,
    Failed { reason: String },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub request: Request,
    pub outcome: Outcome,
}
fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
/// A pasted goal keeps its line breaks (#743) but is one line downstream (commit
/// subjects, `ALFREDO_GOAL`, single-line panels): each run of CR, LF, TAB, VT, FF or
/// NEL (U+0085) becomes one space (#751). ESC, NUL, DEL and other controls stay, to be
/// refused by `goal_refusal`.
pub fn normalise_goal(goal: &str) -> String {
    let lines: Vec<&str> = goal
        .trim()
        .split(['\r', '\n', '\t', '\x0b', '\x0c', '\u{85}'])
        .filter(|line| !line.is_empty())
        .collect();
    lines.join(" ")
}
/// The text after `verb` when whitespace of any kind (space, tab, line break) follows it,
/// so `/plan\nFix it` is `/plan Fix it` and `/planet` is not `/plan`.
pub fn verb_argument<'a>(text: &'a str, verb: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(verb)?;
    rest.starts_with(char::is_whitespace).then_some(rest)
}
/// A `/plan` or `/plan-revise` request as one line, or the refusal that names why not.
pub fn request_text(raw: &str) -> Result<String, String> {
    let request = normalise_goal(raw);
    match goal_refusal("Request", &request, 8192) {
        Some(refusal) => Err(refusal),
        None => Ok(request),
    }
}
/// Why a normalised goal, request or title is refused, naming the rule that failed.
pub fn goal_refusal(what: &str, value: &str, limit: usize) -> Option<String> {
    if value.trim().is_empty() {
        Some(format!("{what} is empty"))
    } else if value.chars().any(char::is_control) {
        Some(format!(
            "{what} has a control character: line breaks are fine but other control characters are not"
        ))
    } else if value.len() > limit {
        Some(format!(
            "{what} is {} bytes; the limit is {limit} bytes",
            value.len()
        ))
    } else {
        None
    }
}
fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if !text(&self.correlation, 160) {
            return Err("Invalid planner command identity".into());
        }
        let valid = match &self.operation {
            Operation::Generate {
                prompt,
                model,
                revision,
                base_sha256,
            } => {
                text(prompt, 8192)
                    && text(model, 200)
                    && *revision <= 4096
                    && base_sha256.as_deref().is_none_or(digest)
            }
            Operation::Revise {
                prompt,
                revision,
                base_sha256,
            } => text(prompt, 8192) && *revision <= 4096 && digest(base_sha256),
            Operation::Architect { origin, revision } => {
                origin.validate()?;
                *revision <= 4096 && origin.review_revision <= *revision
            }
            Operation::Cancel {
                generation,
                draft_sha256,
            } => {
                (generation.is_some() || draft_sha256.is_some())
                    && generation.as_deref().is_none_or(|id| text(id, 160))
                    && draft_sha256.as_deref().is_none_or(digest)
            }
        };
        if !valid {
            return Err("Invalid planner command bounds or source binding".into());
        }
        Ok(())
    }
}
impl Outcome {
    pub fn validate(&self) -> Result<(), String> {
        let valid = match self {
            Self::Generated {
                draft_sha256,
                tasks,
            } => digest(draft_sha256) && (1..=16).contains(tasks),
            Self::Stopped => true,
            Self::Failed { reason } => text(reason, 256),
        };
        if valid {
            Ok(())
        } else {
            Err("Invalid planner outcome".into())
        }
    }
    pub fn failed(reason: &str) -> Self {
        Self::Failed {
            reason: crate::console_command::bounded_reason(reason),
        }
    }
}
