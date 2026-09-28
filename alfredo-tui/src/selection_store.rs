//! Bounded independent selection journal, available before a conversation owner exists.
use crate::{
    selection_command::{Outcome, Phase, Request},
    tasks::{regular_file, StoreLock},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
const LIMIT: usize = 12 * 1024 * 1024;
const MAX_RECORDS: usize = 512;
static TEMP: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub request: Request,
    pub outcome: Outcome,
    pub dispatched: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema_version: u32,
    records: Vec<Record>,
}
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
}
/// Not Clone, serializable, or constructible by callers. Only a new synced record grants admission.
#[derive(Debug)]
pub struct Admission {
    root: PathBuf,
    request: Request,
}
#[derive(Debug)]
pub struct Operation {
    root: PathBuf,
    request: Request,
}
impl Operation {
    pub fn request(&self) -> &Request {
        &self.request
    }
}
impl Store {
    pub fn new(state: &Path) -> Result<Self, String> {
        let state = if state.is_absolute() {
            state.to_owned()
        } else {
            std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(state)
        };
        Ok(Self {
            root: state.join("rust-selection-v1"),
        })
    }
    fn check_path(&self) -> Result<(), String> {
        for path in self.root.ancestors() {
            match fs::symlink_metadata(path) {
                Ok(m) if !m.is_dir() || m.file_type().is_symlink() => {
                    return Err("Selection journal ancestors must be real directories".into())
                }
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
                _ => {}
            }
        }
        Ok(())
    }
    fn lock(&self) -> Result<StoreLock, String> {
        self.check_path()?;
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        self.check_path()?;
        let file = regular_file(&self.root.join("selection.lock"), true, true)?;
        file.try_lock()
            .map_err(|_| "Selection journal busy; no new selection admitted")?;
        Ok(StoreLock(file))
    }
    fn read(&self) -> Result<Snapshot, String> {
        let file = self.root.join("selections.json");
        match fs::symlink_metadata(&file) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Snapshot {
                    schema_version: 1,
                    records: vec![],
                })
            }
            Err(error) => return Err(error.to_string()),
            Ok(_) => {}
        }
        let mut bytes = vec![];
        regular_file(&file, false, false)?
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > LIMIT {
            return Err("Selection journal exceeds size limit".into());
        }
        let state: Snapshot = serde_json::from_slice(&bytes)
            .map_err(|_| "Malformed selection journal; original preserved")?;
        if state.schema_version != 1 || state.records.len() > MAX_RECORDS {
            return Err("Invalid selection journal version or count".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for record in &state.records {
            record.outcome.validate_for(&record.request)?;
            if !ids.insert(&record.request.correlation)
                || (!record.dispatched && record.outcome.phase != Phase::Admitted)
            {
                return Err("Invalid selection journal identity or dispatch state".into());
            }
        }
        Ok(state)
    }
    fn save(&self, state: &Snapshot) -> Result<(), String> {
        let bytes = serde_json::to_vec(state).map_err(|e| e.to_string())?;
        let budget: usize = state
            .records
            .iter()
            .map(|record| {
                serde_json::to_vec(&record.request)
                    .map_or(LIMIT, |b| b.len())
                    .saturating_add(1024)
            })
            .sum();
        if state.records.len() > MAX_RECORDS || bytes.len() > LIMIT || budget > LIMIT {
            return Err("Selection history capacity reached; no selection dispatched".into());
        }
        let temp = self.root.join(format!(
            ".selection-{}-{}.tmp",
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
            let mut file = options.open(&temp).map_err(|e| e.to_string())?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            fs::rename(&temp, self.root.join("selections.json")).map_err(|e| e.to_string())?;
            File::open(&self.root)
                .and_then(|f| f.sync_all())
                .map_err(|e| format!("Selection publication unconfirmed: {e}"))?;
            Ok(())
        })();
        let _ = fs::remove_file(temp);
        result
    }
    pub fn snapshot(&self) -> Result<Vec<Record>, String> {
        self.check_path()?;
        match fs::symlink_metadata(&self.root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(error) => return Err(error.to_string()),
            Ok(_) => {}
        }
        let _lock = self.lock()?;
        Ok(self.read()?.records)
    }
    pub fn admit(&self, request: Request) -> Result<Admission, String> {
        request.validate()?;
        let journal_root = crate::selection::future_path(&self.root)?;
        let target = crate::selection::future_path(&request.choice.target())?;
        if journal_root.starts_with(&target) {
            return Err("Selection state must live outside the requested workspace".into());
        }
        let _lock = self.lock()?;
        let mut state = self.read()?;
        if let Some(old) = state
            .records
            .iter()
            .find(|r| r.request.correlation == request.correlation)
        {
            return Err(if old.request!=request{"Selection identity already used with different input"}else{"Saved selection cannot be replayed; inspect its result and make a new Open or Resume choice"}.into());
        }
        state.records.push(Record {
            request: request.clone(),
            outcome: Outcome::at(Phase::Admitted),
            dispatched: false,
        });
        self.save(&state)?;
        Ok(Admission {
            root: self.root.clone(),
            request,
        })
    }
    pub fn begin(&self, admission: Admission) -> Result<Operation, String> {
        if admission.root != self.root {
            return Err("Selection admission belongs to another journal".into());
        }
        let _lock = self.lock()?;
        let mut state = self.read()?;
        let record = state
            .records
            .iter_mut()
            .find(|r| r.request == admission.request)
            .ok_or("Selection admission no longer exists")?;
        if record.dispatched
            || record.outcome.phase != Phase::Admitted
            || record.outcome.failure.is_some()
        {
            return Err("Selection admission has already been consumed".into());
        }
        record.dispatched = true;
        self.save(&state)?;
        Ok(Operation {
            root: self.root.clone(),
            request: admission.request,
        })
    }
    pub fn record(&self, operation: &Operation, outcome: Outcome) -> Result<(), String> {
        if operation.root != self.root {
            return Err("Selection operation belongs to another journal".into());
        }
        outcome.validate_for(&operation.request)?;
        let _lock = self.lock()?;
        let mut state = self.read()?;
        let record = state
            .records
            .iter_mut()
            .find(|r| r.request == operation.request)
            .ok_or("Selection operation no longer exists")?;
        if !record.dispatched
            || outcome.phase < record.outcome.phase
            || record.outcome.failure.is_some()
        {
            return Err("Selection outcome cannot regress or replay a terminal failure".into());
        }
        record.outcome = outcome;
        self.save(&state)
    }
}
