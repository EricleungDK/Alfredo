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
