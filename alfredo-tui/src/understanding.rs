//! Project-scoped, receipt-validated understanding. Confirmation never dispatches work.
use crate::tasks::{regular_file, StoreLock};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, String>;
const LIMIT: usize = 1024 * 1024;
static TEMP: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Brief {
    pub destination: String,
    pub scope: String,
    pub constraints: String,
    pub uncertainty: String,
}
impl Brief {
    fn validate(&self) -> Result<()> {
        for field in [
            &self.destination,
            &self.scope,
            &self.constraints,
            &self.uncertainty,
        ] {
            if field.trim().is_empty()
                || field.len() > 2048
                || field.chars().any(|c| c.is_control() && c != '\n')
            {
                return Err("Each understanding field needs 1–2048 bytes of text".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Chart,
    WorkThrough,
}
impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Chart => "Chart",
            Self::WorkThrough => "Work-through",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flow {
    pub mode: Mode,
    pub prompt: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Action {
    Enter { flow: Flow },
    Draft { brief: Brief },
    Confirm { draft_revision: u64 },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub correlation: String,
    pub expected_revision: u64,
    pub action: Action,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub request: Request,
    pub actor: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema_version: u32,
    pub workspace: PathBuf,
    pub revision: u64,
    pub draft_revision: u64,
    pub brief: Option<Brief>,
    pub confirmed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<Flow>,
    pub receipts: Vec<Receipt>,
}
impl Snapshot {
    fn apply(&mut self, action: &Action) -> Result<()> {
        match action {
            Action::Enter { flow } => {
                if self.brief.is_some() || self.flow.is_some() || self.revision != 0 {
                    return Err("Continue the existing understanding flow".into());
                }
                if flow.prompt.trim().is_empty()
                    || flow.prompt.len() > crate::model::MAX_DRAFT
                    || flow.prompt.chars().any(|c| c.is_control() && c != '\n')
                {
                    return Err("Wayfinder entry needs bounded prompt text".into());
                }
                let mut end = flow.prompt.len().min(1800);
                while !flow.prompt.is_char_boundary(end) {
                    end -= 1;
                }
                let suffix = if end < flow.prompt.len() {
                    " (excerpt; full request retained in flow)"
                } else {
                    ""
                };
                self.brief = Some(Brief {
                    destination: format!("Requested direction: {}{suffix}", &flow.prompt[..end]),
                    scope: "Not agreed yet; identify what is included and excluded.".into(),
                    constraints:
                        "Not agreed yet; identify compatibility, safety and delivery limits.".into(),
                    uncertainty: "Open questions and acceptance evidence still need discussion."
                        .into(),
                });
                self.flow = Some(flow.clone());
                self.confirmed = false;
                self.draft_revision = self.revision + 1;
            }
            Action::Draft { brief } => {
                brief.validate()?;
                self.brief = Some(brief.clone());
                self.confirmed = false;
                self.draft_revision = self.revision + 1;
            }
            Action::Confirm { draft_revision } => {
                if self.flow.is_some() && self.draft_revision == 1 {
                    return Err("Provide destination, scope, constraints and uncertainty before confirming this new flow".into());
                }
                if self.brief.is_none() || self.confirmed || self.draft_revision != *draft_revision
                {
                    return Err("Confirm the exact pending draft revision".into());
                }
                self.confirmed = true;
            }
        }
        self.revision += 1;
        Ok(())
    }
    pub fn text(&self) -> String {
        let mode = self
            .flow
            .as_ref()
            .map(|flow| format!("Wayfinder / {} · ", flow.mode.label()))
            .unwrap_or_default();
        let heading = format!(
            "{mode}Shared Understanding · revision {} · {}\n",
            self.revision,
            if self.brief.is_none() {
                "Outside flow"
            } else if self.confirmed {
                "Confirmed"
            } else {
                "Pending confirmation"
            }
        );
        match &self.brief {
            None => format!("{heading}\n/scope JSON records destination, scope, constraints and uncertainty.\nNo understanding has been inferred from existing missions."),
            Some(b) => format!("{heading}\nDestination\n{}\n\nScope\n{}\n\nConstraints\n{}\n\nKnown uncertainty\n{}\n\n{}\n{}", b.destination, b.scope, b.constraints, b.uncertainty, if self.confirmed {"Confirmation records agreement only; no task is approved or launched.".into()} else if self.flow.is_some() && self.draft_revision == 1 { "Supply destination, scope, constraints and uncertainty before confirmation. New work remains blocked.".into() } else {format!("Review all fields, then /scope-confirm {}. Saving task plans and starting new work are blocked.",self.draft_revision)}, "A new /scope draft requires fresh confirmation. /tasks returns to task details."),
        }
    }
}
/// Compact provenance retained with a plan; the journal proves current authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub workspace: PathBuf,
    pub revision: u64,
    pub draft_revision: u64,
    pub brief: Option<Brief>,
    pub confirmed: bool,
}
impl Binding {
    pub fn validate(&self) -> Result<()> {
        if !self.workspace.is_absolute() || self.revision > 256 {
            return Err("Invalid plan scope identity or revision".into());
        }
        if let Some(brief) = &self.brief {
            brief.validate()?;
            if self.draft_revision == 0
                || Some(self.revision) != self.draft_revision.checked_add(u64::from(self.confirmed))
            {
                return Err("Invalid plan scope confirmation".into());
            }
        } else if self.revision != 0 || self.draft_revision != 0 || self.confirmed {
            return Err("Outside-flow scope cannot imply confirmation".into());
        }
        Ok(())
    }
}
impl Snapshot {
    pub fn binding(&self) -> Binding {
        Binding {
            workspace: self.workspace.clone(),
            revision: self.revision,
            draft_revision: self.draft_revision,
            brief: self.brief.clone(),
            confirmed: self.confirmed,
        }
    }
}
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    workspace: PathBuf,
}
pub struct Guard<'a> {
    store: &'a Store,
    _lock: StoreLock,
}
impl Guard<'_> {
    pub fn ensure_binding(&self, binding: Option<&Binding>) -> Result<()> {
        let current = self.store.read()?.binding();
        if binding.is_some_and(|binding| binding != &current)
            || (binding.is_none() && current.revision != 0)
        {
            return Err(
                "Plan scope changed or was not recorded; generate and review a fresh /plan".into(),
            );
        }
        Ok(())
    }
    pub fn ensure_open(&self) -> Result<()> {
        let state = self.store.read()?;
        if state.brief.is_some() && !state.confirmed {
            return Err(format!("Shared Understanding pending · review /scope and confirm draft {} before saving plans or starting new work", state.draft_revision));
        }
        Ok(())
    }
}
impl Store {
    pub fn new(state: &Path, workspace: &Path) -> Self {
        Self {
            root: state.join("rust-understanding-v1").join(format!(
                "{:x}",
                Sha256::digest(workspace.as_os_str().as_encoded_bytes())
            )),
            workspace: workspace.into(),
        }
    }
    fn empty(&self) -> Snapshot {
        Snapshot {
            schema_version: 2,
            workspace: self.workspace.clone(),
            revision: 0,
            draft_revision: 0,
            brief: None,
            confirmed: false,
            flow: None,
            receipts: vec![],
        }
    }
    pub fn lock(&self) -> Result<Guard<'_>> {
        for path in self.root.ancestors() {
            match fs::symlink_metadata(path) {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err("Understanding state ancestors must not be symlinks".into())
                }
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
                _ => {}
            }
        }
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let file = regular_file(&self.root.join("scope.lock"), true, true)?;
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            match file.try_lock() {
                Ok(()) => {
                    return Ok(Guard {
                        store: self,
                        _lock: StoreLock(file),
                    })
                }
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(2))
                }
                Err(_) => return Err("Understanding state busy; retry the same request".into()),
            }
        }
    }
    fn read(&self) -> Result<Snapshot> {
        let path = self.root.join("understanding.json");
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(self.empty()),
            Err(e) => return Err(e.to_string()),
            Ok(_) => {}
        }
        let mut bytes = Vec::new();
        regular_file(&path, false, false)?
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > LIMIT {
            return Err("Understanding state exceeds 1 MiB".into());
        }
        let state: Snapshot = serde_json::from_slice(&bytes)
            .map_err(|_| "Malformed understanding state; original preserved")?;
        if !matches!(state.schema_version, 1 | 2)
            || state.workspace != self.workspace
            || state.receipts.len() > 256
            || state.revision != state.receipts.len() as u64
        {
            return Err("Invalid understanding identity/version/bounds".into());
        }
        let mut proof = self.empty();
        let mut ids = std::collections::BTreeSet::new();
        for receipt in &state.receipts {
            if (matches!(receipt.request.action, Action::Enter { .. }) && state.schema_version < 2)
                || receipt.actor != actor(&receipt.request.action)
            {
                return Err("Unknown understanding actor".into());
            }
            let request = &receipt.request;
            validate_request(request)?;
            if request.expected_revision != proof.revision || !ids.insert(&request.correlation) {
                return Err("Invalid understanding receipt sequence".into());
            }
            proof.apply(&request.action)?;
        }
        if (state.schema_version < 2 && state.flow.is_some())
            || proof.flow != state.flow
            || proof.brief != state.brief
            || proof.confirmed != state.confirmed
            || proof.draft_revision != state.draft_revision
        {
            return Err("Understanding projection differs from receipts".into());
        }
        Ok(state)
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        let _guard = self.lock()?;
        self.read()
    }
    fn backup_v1(&self) -> Result<()> {
        let mut original = Vec::new();
        regular_file(&self.root.join("understanding.json"), false, false)?
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut original)
            .map_err(|e| e.to_string())?;
        if original.len() > LIMIT {
            return Err("Understanding backup exceeds limit".into());
        }
        let backup = self.root.join("understanding-v1-backup.json");
        match fs::symlink_metadata(&backup) {
            Ok(_) => {
                let mut existing = Vec::new();
                regular_file(&backup, false, false)?
                    .take((LIMIT + 1) as u64)
                    .read_to_end(&mut existing)
                    .map_err(|e| e.to_string())?;
                if existing != original {
                    return Err("Understanding v1 backup conflict; originals preserved".into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(&backup).map_err(|e| e.to_string())?;
                file.write_all(&original)
                    .and_then(|_| file.sync_all())
                    .map_err(|e| e.to_string())?;
            }
            Err(error) => return Err(error.to_string()),
        }
        Ok(())
    }
    pub fn transact(&self, request: Request) -> Result<Snapshot> {
        validate_request(&request)?;
        let _guard = self.lock()?;
        self.commit(request)
    }
    /// Convenience entry for callers that already authorize the exact operation.
    pub fn enter(&self, correlation: String, flow: Flow) -> Result<(Snapshot, bool)> {
        self.enter_prepared(&Request {
            correlation,
            expected_revision: 0,
            action: Action::Enter { flow },
        })
    }
    /// Atomically admit the exact prepared first-contact request. The boolean
    /// means this request has a receipt (new or replayed), not merely that a flow exists.
    pub fn enter_prepared(&self, request: &Request) -> Result<(Snapshot, bool)> {
        validate_request(request)?;
        if !matches!(request.action, Action::Enter { .. }) {
            return Err("Prepared flow entry requires an Enter operation".into());
        }
        let _guard = self.lock()?;
        let state = self.read()?;
        if let Some(receipt) = state
            .receipts
            .iter()
            .find(|receipt| receipt.request.correlation == request.correlation)
        {
            if receipt.request != *request {
                return Err("Understanding correlation already used for another request".into());
            }
            return Ok((state, true));
        }
        if state.brief.is_some() {
            return Ok((state, false));
        }
        Ok((self.commit(request.clone())?, true))
    }
    fn commit(&self, request: Request) -> Result<Snapshot> {
        let mut state = self.read()?;
        let legacy = state.schema_version == 1;
        if let Some(old) = state
            .receipts
            .iter()
            .find(|r| r.request.correlation == request.correlation)
        {
            if old.request != request {
                return Err("Understanding correlation already used for another request".into());
            }
            return Ok(state);
        }
        if request.expected_revision != state.revision {
            return Err("Understanding changed; review /scope before retrying".into());
        }
        if state.receipts.len() == 256 {
            return Err("Understanding receipt capacity reached".into());
        }
        let drafting = matches!(request.action, Action::Draft { .. });
        if drafting && state.receipts.len() + 2 > 256 {
            return Err("Understanding capacity must reserve confirmation".into());
        }
        state.apply(&request.action)?;
        state.schema_version = 2;
        state.receipts.push(Receipt {
            actor: actor(&request.action).into(),
            request,
        });
        if drafting {
            let mut reserved = state.clone();
            let action = Action::Confirm {
                draft_revision: state.draft_revision,
            };
            reserved.apply(&action)?;
            reserved.receipts.push(Receipt {
                request: Request {
                    correlation: "\"".repeat(160),
                    expected_revision: state.revision,
                    action,
                },
                actor: "mission-commander".into(),
            });
            if serde_json::to_vec(&reserved)
                .map_err(|e| e.to_string())?
                .len()
                > LIMIT
            {
                return Err("Understanding capacity must reserve confirmation bytes".into());
            }
        }
        let bytes = serde_json::to_vec(&state).map_err(|e| e.to_string())?;
        if bytes.len() > LIMIT {
            return Err("Understanding state exceeds 1 MiB".into());
        }
        if legacy {
            self.backup_v1()?;
        }
        let temporary = self.root.join(format!(
            ".scope-{}-{}.tmp",
            std::process::id(),
            TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            fs::rename(&temporary, self.root.join("understanding.json"))
                .map_err(|e| e.to_string())?;
            File::open(&self.root)
                .and_then(|f| f.sync_all())
                .map_err(|e| {
                    format!("Understanding acknowledgment uncertain: {e}; retry exact request")
                })?;
            Ok(state)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}
pub fn validate_request(request: &Request) -> Result<()> {
    if request.correlation.trim().is_empty()
        || request.correlation.len() > 160
        || request.correlation.chars().any(char::is_control)
        || request.expected_revision > 256
    {
        return Err("Invalid understanding identity or revision".into());
    }
    match &request.action {
        Action::Enter { flow } => {
            if request.expected_revision != 0
                || flow.prompt.trim().is_empty()
                || flow.prompt.len() > crate::model::MAX_DRAFT
                || flow.prompt.chars().any(|c| c.is_control() && c != '\n')
            {
                return Err("Wayfinder entry needs bounded prompt text at revision zero".into());
            }
        }
        Action::Draft { brief } => brief.validate()?,
        Action::Confirm { draft_revision } => {
            if *draft_revision == 0
                || *draft_revision > 256
                || request.expected_revision != *draft_revision
            {
                return Err("Confirmation must bind its exact draft revision".into());
            }
        }
    }
    Ok(())
}

fn actor(action: &Action) -> &'static str {
    match action {
        Action::Enter { .. } => "wayfinder-alfredo",
        _ => "mission-commander",
    }
}
