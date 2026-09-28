//! Explicit reviewer assertions, never independently inferred check success.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub criterion: u64,
    pub met: bool,
    pub note: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assessment {
    pub accept: bool,
    pub reason: String,
    pub criteria: Vec<Criterion>,
}
impl Assessment {
    pub fn validate(&self) -> Result<(), String> {
        let text = |value: &str, limit| {
            !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
        };
        if !text(&self.reason, 2048) || self.criteria.len() > 16 {
            return Err(
                "Review needs a nonempty reason (up to 2048 bytes) and at most 16 criteria".into(),
            );
        }
        for (index, item) in self.criteria.iter().enumerate() {
            if item.criterion != index as u64 + 1 || !text(&item.note, 1024) {
                return Err("Review criteria must be ordered 1..N with a nonempty evidence note (up to 1024 bytes)".into());
            }
            if self.accept && !item.met {
                return Err(
                    "Acceptance requires every recorded criterion to be assessed as met".into(),
                );
            }
        }
        Ok(())
    }
    pub fn validate_contract(&self, contract: &[String]) -> Result<(), String> {
        self.validate()?;
        if self.criteria.len() != contract.len() {
            return Err("Review must assess each recorded acceptance criterion exactly once; inspect /evidence ID".into());
        }
        Ok(())
    }
    pub fn summary(&self) -> String {
        let mut result = format!(
            "Reviewer {}: {}",
            if self.accept { "accepted" } else { "rejected" },
            self.reason
        );
        for item in &self.criteria {
            result.push_str(&format!(
                "\nCriterion {} · {} · {}",
                item.criterion,
                if item.met { "met" } else { "not met" },
                item.note
            ));
        }
        result
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Approved,
    ApprovedWithLimitations,
    NeedsRepair,
    NeedsHumanReview,
    Rejected,
}
impl Outcome {
    pub fn approves(self) -> bool {
        matches!(self, Self::Approved | Self::ApprovedWithLimitations)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Approved => "Approved",
            Self::ApprovedWithLimitations => "Approved with limitations",
            Self::NeedsRepair => "Needs repair",
            Self::NeedsHumanReview => "Needs human review",
            Self::Rejected => "Rejected",
        }
    }
    pub fn task_label(self) -> &'static str {
        match self {
            Self::Approved => "Accepted",
            Self::ApprovedWithLimitations => "Accepted (limited)",
            Self::NeedsRepair => "Needs repair",
            Self::NeedsHumanReview => "Needs human review",
            Self::Rejected => "Rejected",
        }
    }
}
/// Reviewer-declared risk; absence means unclassified, not proven safe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewRisk {
    Critical,
    Security,
    MergeRisk,
}
impl ReviewRisk {
    pub fn label(self) -> &'static str {
        match self {
            Self::Critical => "Critical",
            Self::Security => "Security",
            Self::MergeRisk => "Merge risk",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureKind {
    Architecture,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<FailureKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<ReviewRisk>,
    pub outcome: Outcome,
    pub reason: String,
    pub criteria: Vec<Criterion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
}
impl Decision {
    pub fn proposes_repair(&self) -> bool {
        self.risk.is_none() && matches!(self.outcome, Outcome::NeedsRepair | Outcome::Rejected)
    }
    pub fn requires_human_review(&self) -> bool {
        self.risk.is_some() || self.outcome == Outcome::NeedsHumanReview
    }
    fn assessment(&self) -> Assessment {
        Assessment {
            accept: self.outcome.approves(),
            reason: self.reason.clone(),
            criteria: self.criteria.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        self.assessment().validate()?;
        if self.failure.is_some() && self.outcome.approves() {
            return Err("An approved review cannot declare unresolved failure".into());
        }
        if self.risk.is_some() && self.outcome.approves() {
            return Err(
                "Unresolved critical, security or merge risk requires human review before approval"
                    .into(),
            );
        }
        let unique: std::collections::BTreeSet<_> =
            self.limitations.iter().map(|s| s.trim()).collect();
        if self.limitations.len() > 8
            || unique.len() != self.limitations.len()
            || self
                .limitations
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 1024 || s.chars().any(char::is_control))
            || (self.outcome == Outcome::ApprovedWithLimitations) == self.limitations.is_empty()
        {
            return Err("Only Approved with limitations requires 1–8 distinct nonempty limitation lines (up to 1024 bytes each)".into());
        }
        Ok(())
    }
    pub fn validate_contract(&self, contract: &[String]) -> Result<(), String> {
        self.validate()?;
        self.assessment().validate_contract(contract)
    }
    pub fn summary(&self) -> String {
        let assessment = self.assessment().summary();
        let mut text = format!(
            "Reviewer outcome: {}\n{}",
            self.outcome.label(),
            assessment
                .split_once(": ")
                .map_or(assessment.as_str(), |(_, rest)| rest)
        );
        if let Some(risk) = self.risk {
            text.push_str(&format!(
                "\nRisk: {} · escalated to human review",
                risk.label()
            ));
        }
        if self.failure == Some(FailureKind::Architecture) {
            text.push_str("\nFailure: architecture");
        }
        for limitation in &self.limitations {
            text.push_str(&format!("\nLimitation: {limitation}"));
        }
        text
    }
}
