//! Trusted binding between a repeated failure and a proposed Architect revision.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    pub task: u64,
    pub review_revision: u64,
    pub run: String,
    pub evidence_sha256: String,
}
impl Origin {
    pub fn validate(&self) -> Result<(), String> {
        if self.task == 0
            || self.review_revision == 0
            || self.run.len() > 100
            || !self.run.starts_with("task-")
            || !self
                .run
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || self.evidence_sha256.len() != 64
            || !self.evidence_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("Invalid Architect source binding".into());
        }
        Ok(())
    }
}
pub struct Context {
    pub baseline: String,
    pub origin: Origin,
    pub prompt: String,
    pub model: String,
    pub reference: String,
    pub revision: u64,
}
