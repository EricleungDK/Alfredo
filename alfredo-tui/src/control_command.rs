//! Process-bound operator actions; local acknowledgment never proves a worker result.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub correlation: String,
    pub controller: String,
    pub operation: Operation,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Operation {
    CancelWorker {
        task: u64,
        start_correlation: String,
        /// Revision of the expected Start receipt, not its preceding snapshot.
        expected_start_revision: u64,
    },
    Dispatch {
        enabled: bool,
        expected_epoch: u64,
        scope_revision_for_on: Option<u64>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Outcome {
    CancellationRequested,
    DispatchChanged { enabled: bool },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub request: Request,
    pub outcome: Outcome,
}
fn identity(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 160 && !value.chars().any(char::is_control)
}
impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if !identity(&self.correlation) || !identity(&self.controller) {
            return Err("Invalid controller command identity".into());
        }
        let valid = match &self.operation {
            Operation::CancelWorker {
                task,
                start_correlation,
                expected_start_revision,
            } => {
                *task > 0
                    && *task <= 256
                    && identity(start_correlation)
                    && (1..=4096).contains(expected_start_revision)
            }
            Operation::Dispatch {
                enabled,
                expected_epoch,
                scope_revision_for_on,
            } => {
                *expected_epoch < u64::MAX
                    && if *enabled {
                        scope_revision_for_on.is_some_and(|revision| revision <= 256)
                    } else {
                        scope_revision_for_on.is_none()
                    }
            }
        };
        if valid {
            Ok(())
        } else {
            Err("Invalid controller command bounds or target".into())
        }
    }
}
impl Outcome {
    pub fn validate_for(&self, request: &Request) -> Result<(), String> {
        request.validate()?;
        match (self, &request.operation) {
            (Self::CancellationRequested, Operation::CancelWorker { .. }) => Ok(()),
            (
                Self::DispatchChanged { enabled },
                Operation::Dispatch {
                    enabled: requested, ..
                },
            ) if enabled == requested => Ok(()),
            _ => Err("Controller outcome does not match its request".into()),
        }
    }
}
