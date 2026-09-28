//! Bounded local conversation snapshots. No inference or task effect is replayed.
use crate::{
    model::{App, Session, Status, MAX_DRAFT, MAX_MESSAGES, MAX_SESSIONS, MAX_TEXT},
    tasks::{regular_file, StoreLock, TaskStore},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::{runtime::Runtime, task::JoinHandle};

const LIMIT: usize = 12 * 1024 * 1024; // JSON escaping can expand bounded text sixfold.
static TEMP: AtomicU64 = AtomicU64::new(0);
type Result<T> = std::result::Result<T, String>;

/// Presentation preferences only; never an execution or review instruction.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskView {
    pub visible: bool,
    pub selected: Option<u64>,
    pub query: String,
}
impl TaskView {
    pub fn validate(&self) -> Result<()> {
        if self.query.len() > 200
            || self.query.chars().any(char::is_control)
            || self.selected == Some(0)
        {
            return Err("Invalid saved task view".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema_version: u32,
    pub namespace: String,
    pub selected: usize,
    pub sessions: Vec<Session>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_view: Option<TaskView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_draft: Option<crate::planner::SavedDraft>,
}
impl Snapshot {
    pub fn capture(app: &App, namespace: &str) -> Self {
        Self {
            schema_version: 17,
            task_view: None,
            plan_draft: None,
            namespace: namespace.into(),
            selected: app.selected,
            sessions: app.sessions.iter().map(Session::checkpoint).collect(),
        }
    }
    pub(crate) fn validate(&self, namespace: &str) -> Result<()> {
        if !matches!(self.schema_version, 1..=17)
            || (self.schema_version == 1 && self.task_view.is_some())
            || (self.schema_version < 5 && self.plan_draft.is_some())
            || self.namespace != namespace
            || self.sessions.is_empty()
            || self.sessions.len() > MAX_SESSIONS
            || self.selected >= self.sessions.len()
        {
            return Err(
                "Invalid conversation version, identity or selection; original file preserved"
                    .into(),
            );
        }
        if let Some(view) = &self.task_view {
            view.validate()?;
        }
        if let Some(draft) = &self.plan_draft {
            draft.validate()?;
            if let Some(origin) = &draft.origin {
                let owner = self
                    .sessions
                    .iter()
                    .flat_map(|session| session.commands())
                    .find(|command| command.intent.planner_request() == Some(origin));
                if self.schema_version < 12
                    || owner.is_none()
                    || (self.schema_version < 15
                        && owner.is_some_and(|command| {
                            matches!(
                                command.intent,
                                crate::command_intent::Intent::ArchitectDraft { .. }
                            )
                        }))
                {
                    return Err("Planner draft origin requires its saved command and conversation schema v12".into());
                }
                if let Some(crate::console_command::ConsoleCommand {
                    state:
                        crate::console_command::CommandState::Planner {
                            outcome: outcome @ crate::planner_command::Outcome::Generated { .. },
                        },
                    ..
                }) = owner
                {
                    if draft.outcome_for(origin).as_ref() != Some(outcome) {
                        return Err("Planner draft and originating result disagree".into());
                    }
                }
            }
            if self.schema_version < 7 && draft.plan.architecture.is_some() {
                return Err("Architect draft provenance requires conversation schema v7".into());
            }
            if self.schema_version < 6
                && draft
                    .plan
                    .tasks
                    .iter()
                    .any(|step| !step.acceptance.is_empty())
            {
                return Err("Saved acceptance criteria require conversation schema v6".into());
            }
        }
        let mut command_ids = std::collections::BTreeSet::new();
        let mut planner_ids = std::collections::BTreeSet::new();
        let mut scope_ids = std::collections::BTreeSet::new();
        let mut selection_ids = std::collections::BTreeSet::new();
        for (session_index, session) in self.sessions.iter().enumerate() {
            if !session.valid_reading(self.schema_version) {
                return Err("Invalid conversation reading anchor".into());
            }
            if !session.valid_sources(self.schema_version) {
                return Err("Invalid conversation response sources".into());
            }
            if !session.valid_task_receipts(self.schema_version) {
                return Err("Invalid conversation task receipt references".into());
            }
            if !session.valid_commands(self.schema_version) {
                return Err("Invalid saved console commands".into());
            }
            if session
                .commands()
                .iter()
                .any(|command| !command_ids.insert(&command.id))
            {
                return Err("Console command origin is duplicated across sessions".into());
            }
            for request in session
                .commands()
                .iter()
                .filter_map(|command| command.intent.planner_request())
            {
                if !planner_ids.insert(&request.correlation) {
                    return Err("Planner command identity is duplicated across sessions".into());
                }
            }
            for request in session
                .commands()
                .iter()
                .filter_map(|command| command.intent.scope_request())
            {
                if !scope_ids.insert(&request.correlation) {
                    return Err("Scope command identity is duplicated across sessions".into());
                }
            }
            for command in session.commands() {
                let Some(request) = command.intent.selection_request() else {
                    continue;
                };
                if !selection_ids.insert(&request.correlation) {
                    return Err("Selection identity is duplicated in conversation history".into());
                }
                match &command.intent {
                    crate::command_intent::Intent::Selection { .. } => {
                        if !matches!(&request.origin, crate::selection_command::Origin::Conversation { conversation, session, .. } if conversation == &self.namespace && *session == session_index)
                        {
                            return Err("Selection command does not match its originating session and conversation".into());
                        }
                    }
                    crate::command_intent::Intent::SelectionArrival { .. } => {
                        if request.conversation != self.namespace {
                            return Err(
                                "Selection arrival has a different destination conversation".into(),
                            );
                        }
                        if matches!(&request.origin, crate::selection_command::Origin::Conversation { workspace, mission, conversation, .. } if workspace == &request.choice.target() && mission == request.choice.mission.name() && conversation == &request.conversation)
                        {
                            return Err("Same-target selection must not create an arrival".into());
                        }
                    }
                    _ => unreachable!("selection_request only returns selection operations"),
                }
            }
            if session.model.trim().is_empty()
                || session.model.len() > 200
                || session.model.chars().any(char::is_control)
                || session.draft.len() > MAX_DRAFT
                || !session.valid_cursor()
                || session.attempt == u64::MAX
                || session.attempt < (session.messages.len() / 2) as u64
                || session.draft.chars().any(|c| c.is_control() && c != '\n')
                || session.messages.len() > MAX_MESSAGES
                || session
                    .messages
                    .iter()
                    .map(|m| m.content.len())
                    .sum::<usize>()
                    > MAX_TEXT
            {
                return Err("Conversation exceeds supported bounds".into());
            }
            if session.messages.len() % 2 != 0
                || session
                    .messages
                    .iter()
                    .enumerate()
                    .any(|(index, m)| m.role != if index % 2 == 0 { "user" } else { "assistant" })
            {
                return Err("Conversation role order is invalid".into());
            }
            if (session.messages.is_empty() && session.status != Status::Ready)
                || (!session.messages.is_empty()
                    && (session.status == Status::Ready || session.attempt == 0))
            {
                return Err("Conversation lifecycle does not match its turns".into());
            }
            if let Status::Failed(reason) = &session.status {
                if reason.len() > MAX_TEXT {
                    return Err("Conversation failure exceeds bounds".into());
                }
            }
        }
        Ok(())
    }
    pub fn restore(mut self) -> App {
        for session in &mut self.sessions {
            session.restore_history();
            session.restore_commands();
            if let Some(draft) = &self.plan_draft {
                let recovered: Vec<_> = session
                    .commands()
                    .iter()
                    .filter_map(|command| {
                        if !matches!(
                            command.state,
                            crate::console_command::CommandState::Unknown { .. }
                        ) {
                            return None;
                        }
                        let request = command.intent.planner_request()?;
                        draft
                            .outcome_for(request)
                            .map(|outcome| (command.id.clone(), outcome))
                    })
                    .collect();
                for (id, outcome) in recovered {
                    session.set_command_state(
                        &id,
                        crate::console_command::CommandState::Planner { outcome },
                    );
                }
            }
            if session.status.active() {
                session.status = Status::Failed(
                    "Interrupted before saved completion; partial reply retained. Retry explicitly"
                        .into(),
                );
            }
        }
        let mut app = App::new(self.sessions[0].model.clone());
        app.sessions = self.sessions;
        app.selected = self.selected;
        app.notice = "Conversations restored; interrupted requests are never replayed".into();
        app
    }
}

pub struct ConversationStore {
    root: PathBuf,
    path: PathBuf,
    namespace: String,
    workspace: PathBuf,
    mission: String,
    _owner: StoreLock,
}
impl ConversationStore {
    pub fn open(tasks: &TaskStore, namespace: &str) -> Result<Self> {
        if namespace.trim().is_empty()
            || namespace.len() > 120
            || namespace.chars().any(char::is_control)
        {
            return Err("Conversation name must contain 1–120 bytes without controls".into());
        }
        let root = tasks.conversation_directory()?;
        let digest = format!("{:x}", Sha256::digest(namespace.as_bytes()));
        let owner = regular_file(
            &root.join(format!("conversations-{digest}.lock")),
            true,
            true,
        )?;
        owner
            .try_lock()
            .map_err(|_| "Conversation namespace is in use; choose another --conversation NAME")?;
        Ok(Self {
            path: root.join(format!("conversations-{digest}.json")),
            root,
            namespace: namespace.into(),
            workspace: tasks.identity().0.to_owned(),
            mission: tasks.identity().1.to_owned(),
            _owner: StoreLock(owner),
        })
    }
    fn validate_selection_identity(&self, snapshot: &Snapshot) -> Result<()> {
        for command in snapshot
            .sessions
            .iter()
            .flat_map(|session| session.commands())
        {
            let Some(request) = command.intent.selection_request() else {
                continue;
            };
            let matches = match &command.intent {
                crate::command_intent::Intent::Selection { .. } => matches!(&request.origin,
                    crate::selection_command::Origin::Conversation { workspace, mission, .. }
                        if workspace == &self.workspace && mission == &self.mission),
                crate::command_intent::Intent::SelectionArrival { .. } => {
                    request.choice.target() == self.workspace
                        && request.choice.mission.name() == self.mission
                }
                _ => false,
            };
            if !matches {
                return Err("Selection history belongs to a different workspace or mission".into());
            }
        }
        Ok(())
    }
    pub fn load(&self) -> Result<Option<Snapshot>> {
        match fs::symlink_metadata(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
            Ok(_) => {}
        }
        let mut bytes = Vec::new();
        regular_file(&self.path, false, false)?
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > LIMIT {
            return Err("Conversation file exceeds 12 MiB; original preserved".into());
        }
        let snapshot: Snapshot = serde_json::from_slice(&bytes)
            .map_err(|_| "Malformed conversation state; original preserved")?;
        snapshot.validate(&self.namespace)?;
        self.validate_selection_identity(&snapshot)?;
        Ok(Some(snapshot))
    }
    pub fn save(&self, snapshot: &Snapshot) -> Result<()> {
        snapshot.validate(&self.namespace)?;
        self.validate_selection_identity(snapshot)?;
        let bytes = serde_json::to_vec(snapshot).map_err(|e| e.to_string())?;
        if bytes.len() > LIMIT {
            return Err("Conversation file exceeds 12 MiB".into());
        }
        if let Some(previous) = self.load()? {
            if previous.schema_version > snapshot.schema_version {
                return Err("Conversation schema downgrade refused; original preserved".into());
            }
            if previous.schema_version < snapshot.schema_version {
                let mut original = Vec::new();
                regular_file(&self.path, false, false)?
                    .take((LIMIT + 1) as u64)
                    .read_to_end(&mut original)
                    .map_err(|e| e.to_string())?;
                if original.len() > LIMIT {
                    return Err("Legacy conversation exceeds bounds".into());
                }
                let backup = self
                    .path
                    .with_extension(format!("v{}-backup", previous.schema_version));
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                match options.open(&backup) {
                    Ok(mut file) => file
                        .write_all(&original)
                        .and_then(|_| file.sync_all())
                        .map_err(|e| e.to_string())?,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        let mut retained = Vec::new();
                        regular_file(&backup, false, false)?
                            .take((LIMIT + 1) as u64)
                            .read_to_end(&mut retained)
                            .map_err(|e| e.to_string())?;
                        if retained != original {
                            return Err(
                                "Legacy conversation backup differs; original preserved".into()
                            );
                        }
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
        }
        // Reject hostile/special replacement targets instead of following them.
        if fs::symlink_metadata(&self.path).is_ok() {
            regular_file(&self.path, false, false)?;
        }
        let temporary = self.root.join(format!(
            ".conversation-{}-{}.tmp",
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
            fs::rename(&temporary, &self.path).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            File::open(&self.root)
                .and_then(|dir| dir.sync_all())
                .map_err(|e| e.to_string())?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

pub struct Autosave {
    store: Arc<ConversationStore>,
    pending: Option<JoinHandle<Result<Snapshot>>>,
    saved: Option<Snapshot>,
}
impl Autosave {
    pub fn new(store: ConversationStore) -> Self {
        Self {
            store: Arc::new(store),
            pending: None,
            saved: None,
        }
    }
    /// A completed older checkpoint cannot authorize a newer or changed intent.
    /// Only the exact pending entry in a successfully synced snapshot qualifies.
    pub fn contains_saved_command(
        &self,
        session_index: usize,
        command: &crate::console_command::ConsoleCommand,
    ) -> bool {
        matches!(command.state, crate::console_command::CommandState::Pending)
            && self
                .saved
                .as_ref()
                .and_then(|snapshot| snapshot.sessions.get(session_index))
                .is_some_and(|session| session.commands().iter().any(|saved| saved == command))
    }
    pub fn checkpoint(
        &mut self,
        runtime: &Runtime,
        app: &App,
        view: TaskView,
        plan: Option<crate::planner::SavedDraft>,
    ) -> Result<bool> {
        let mut acknowledged = false;
        if self.pending.as_ref().is_some_and(|job| job.is_finished()) {
            let result = runtime
                .block_on(self.pending.take().unwrap())
                .map_err(|_| "Conversation save worker stopped")??;
            self.saved = Some(result);
            acknowledged = true;
        }
        if self.pending.is_none() {
            let mut snapshot = Snapshot::capture(app, &self.store.namespace);
            snapshot.task_view = Some(view);
            snapshot.plan_draft = plan;
            if self.saved.as_ref() != Some(&snapshot) {
                let store = self.store.clone();
                self.pending = Some(runtime.spawn_blocking(move || {
                    store.save(&snapshot)?;
                    Ok(snapshot)
                }));
            }
        }
        Ok(acknowledged)
    }
    pub fn finish(
        &mut self,
        runtime: &Runtime,
        app: &App,
        view: TaskView,
        plan: Option<crate::planner::SavedDraft>,
    ) -> Result<()> {
        // Join the older save before final publication, preventing stale overwrite.
        if let Some(job) = self.pending.take() {
            let _ = runtime.block_on(job);
        }
        let mut snapshot = Snapshot::capture(app, &self.store.namespace);
        snapshot.task_view = Some(view);
        snapshot.plan_draft = plan;
        self.store.save(&snapshot)?;
        self.saved = Some(snapshot);
        Ok(())
    }
}
