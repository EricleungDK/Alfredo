//! Exact workspace-selection intent and local milestones. These never approve task effects.
use serde::{Deserialize, Serialize};
use std::{
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
static IDS: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum WorkspaceChoice {
    Existing { path: PathBuf },
    Create { parent: PathBuf, name: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum MissionChoice {
    Resume { name: String },
    StartNew { name: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub workspace: WorkspaceChoice,
    pub mission: MissionChoice,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Origin {
    Startup,
    Conversation {
        workspace: PathBuf,
        mission: String,
        conversation: String,
        session: usize,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub correlation: String,
    pub origin: Origin,
    pub choice: Choice,
    pub conversation: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Admitted,
    RepositoryReady,
    MissionReady,
    TargetLoaded,
    HandoffPrepared,
    Selected,
    AlreadyCurrent,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub phase: Phase,
    pub failure: Option<String>,
}
fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
fn path(value: &Path) -> bool {
    value.is_absolute()
        && value.to_str().is_some_and(|s| text(s, 4096))
        && !value
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
}
impl WorkspaceChoice {
    pub fn path(&self) -> PathBuf {
        match self {
            Self::Existing { path } => path.clone(),
            Self::Create { parent, name } => parent.join(name),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = match self {
            Self::Existing { path: p } => path(p),
            Self::Create { parent, name } => {
                path(parent)
                    && text(name, 255)
                    && Path::new(name).components().count() == 1
                    && matches!(
                        Path::new(name).components().next(),
                        Some(Component::Normal(_))
                    )
                    && !name.contains(['/', '\\'])
                    && path(&parent.join(name))
            }
        };
        if valid {
            Ok(())
        } else {
            Err("Invalid exact workspace choice".into())
        }
    }
}
impl MissionChoice {
    pub fn name(&self) -> &str {
        match self {
            Self::Resume { name } | Self::StartNew { name } => name,
        }
    }
    pub fn start_new(&self) -> bool {
        matches!(self, Self::StartNew { .. })
    }
    pub fn validate(&self) -> Result<(), String> {
        if text(self.name(), 120) {
            Ok(())
        } else {
            Err("Mission name must contain 1–120 bytes without controls".into())
        }
    }
}
impl Choice {
    pub fn target(&self) -> PathBuf {
        self.workspace.path()
    }
    pub fn validate(&self) -> Result<(), String> {
        self.workspace.validate()?;
        self.mission.validate()?;
        if matches!(self.workspace, WorkspaceChoice::Create { .. }) && !self.mission.start_new() {
            return Err("A new repository needs a new mission".into());
        }
        Ok(())
    }
}
impl Request {
    pub fn new(origin: Origin, choice: Choice, conversation: String) -> Result<Self, String> {
        let request = Self {
            correlation: format!(
                "selection-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                IDS.fetch_add(1, Ordering::Relaxed)
            ),
            origin,
            choice,
            conversation,
        };
        request.validate()?;
        Ok(request)
    }
    pub fn validate(&self) -> Result<(), String> {
        self.choice.validate()?;
        if !text(&self.correlation, 160)
            || !text(&self.conversation, 120)
            || serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 32 * 1024
        {
            return Err("Selection identity or size exceeds bounds".into());
        }
        if let Origin::Conversation {
            workspace,
            mission,
            conversation,
            session,
        } = &self.origin
        {
            if !path(workspace)
                || !text(mission, 120)
                || !text(conversation, 120)
                || *session >= crate::model::MAX_SESSIONS
            {
                return Err("Invalid selection origin".into());
            }
        }
        Ok(())
    }
    pub fn already_current(&self) -> bool {
        matches!((&self.origin,&self.choice.workspace,&self.choice.mission),(Origin::Conversation{workspace,mission,conversation,..},WorkspaceChoice::Existing{path},MissionChoice::Resume{name}) if workspace==path && mission==name && conversation==&self.conversation)
    }
}
impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Admitted => "intent admitted",
            Self::RepositoryReady => "repository ready",
            Self::MissionReady => "mission ready",
            Self::TargetLoaded => "target loaded",
            Self::HandoffPrepared => "handoff prepared",
            Self::Selected => "workspace selected",
            Self::AlreadyCurrent => "already current",
        }
    }
}
impl Outcome {
    pub fn at(phase: Phase) -> Self {
        Self {
            phase,
            failure: None,
        }
    }
    pub fn failed(mut self, reason: &str) -> Self {
        self.failure = Some(crate::console_command::bounded_reason(reason));
        self
    }
    pub fn validate_for(&self, request: &Request) -> Result<(), String> {
        request.validate()?;
        if self.failure.as_deref().is_some_and(|s| !text(s, 256))
            || (self.phase == Phase::AlreadyCurrent && !request.already_current())
        {
            return Err("Invalid selection outcome".into());
        }
        Ok(())
    }
}
