//! Durable presentation intent. Only canonical receipts acknowledge effects.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CommandState {
    Pending,
    Submitted,
    Selection {
        outcome: crate::selection_command::Outcome,
    },
    Control {
        outcome: crate::control_command::Outcome,
    },
    Planner {
        outcome: crate::planner_command::Outcome,
    },
    Unknown {
        reason: String,
    },
    Refused {
        reason: String,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsoleCommand {
    pub id: String,
    pub sequence: u64,
    pub after_messages: usize,
    pub attempt: u64,
    pub text: String,
    pub intent: crate::command_intent::Intent,
    pub state: CommandState,
}
impl ConsoleCommand {
    pub fn identity(intent: &crate::command_intent::Intent) -> String {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(intent).expect("serializable intent"))
        )
    }
    pub fn valid(&self, messages: usize) -> bool {
        self.id == Self::identity(&self.intent)
            && self.sequence > 0
            && self.attempt > 0
            && self.attempt < u64::MAX
            && self.after_messages <= messages
            && self.after_messages.is_multiple_of(2)
            && !self.text.trim().is_empty()
            && self.text.len() <= crate::model::MAX_DRAFT
            && !self.text.chars().any(|c| c.is_control() && c != '\n')
            && self.intent.validate().is_ok()
            && (!matches!(
                self.intent,
                crate::command_intent::Intent::SelectionArrival { .. }
            ) || matches!(&self.state,
                    CommandState::Selection { outcome } if matches!(outcome.phase, crate::selection_command::Phase::HandoffPrepared | crate::selection_command::Phase::Selected))
                || matches!(self.state, CommandState::Unknown { .. }))
            && match &self.state {
                CommandState::Pending | CommandState::Submitted => true,
                CommandState::Selection { outcome } => self
                    .intent
                    .selection_request()
                    .is_some_and(|request| outcome.validate_for(request).is_ok()),
                CommandState::Control { outcome } => match &self.intent {
                    crate::command_intent::Intent::Control { request } => {
                        outcome.validate_for(request).is_ok()
                    }
                    _ => false,
                },
                CommandState::Planner { outcome } => match self.intent.planner_request() {
                    Some(request) => {
                        outcome.validate().is_ok()
                            && !(matches!(
                                outcome,
                                crate::planner_command::Outcome::Generated { .. }
                            ) && matches!(
                                request.operation,
                                crate::planner_command::Operation::Cancel { .. }
                            ))
                    }
                    _ => false,
                },
                CommandState::Unknown { reason } | CommandState::Refused { reason } => {
                    !reason.is_empty()
                        && reason.len() <= 256
                        && !reason.chars().any(char::is_control)
                }
            }
    }
}
pub fn bounded_reason(reason: &str) -> String {
    let mut result = String::new();
    for c in reason.chars().filter(|c| !c.is_control()) {
        if result.len() + c.len_utf8() > 256 {
            break;
        }
        result.push(c);
    }
    if result.is_empty() {
        "Outcome unconfirmed; inspect canonical activity".into()
    } else {
        result
    }
}
