//! Advisory namespace discovery. Entries carry names, never task or execution authority.
use crate::tasks::{regular_file, TaskStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static TEMP: AtomicU64 = AtomicU64::new(0);
const MAX_SCAN: usize = 1024;
const MAX_BYTES: usize = 16 * 1024 * 1024;
type Result<T> = std::result::Result<T, String>;
#[derive(Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Identity {
    schema_version: u32,
    workspace: PathBuf,
    mission: String,
}
impl Identity {
    fn valid(&self) -> bool {
        self.schema_version == 1
            && self.workspace.is_absolute()
            && self
                .workspace
                .to_str()
                .is_some_and(|s| s.len() <= 4096 && !s.chars().any(char::is_control))
            && !self.mission.trim().is_empty()
            && self.mission.len() <= 120
            && !self.mission.chars().any(char::is_control)
    }
    fn encode(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > 8192 {
            return Err("Mission identity exceeds 8 KiB encoded bound".into());
        }
        Ok(bytes)
    }
    fn namespace(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(&self.workspace, &self.mission))
                    .expect("serializable identity")
            )
        )
    }
}

pub fn remember(store: &TaskStore, workspace: &Path, mission: &str) -> Result<()> {
    let identity = Identity {
        schema_version: 1,
        workspace: workspace.into(),
        mission: mission.into(),
    };
    if !identity.valid() {
        return Err("Invalid discovery identity".into());
    }
    let root = store.conversation_directory()?;
    if root.file_name().and_then(|s| s.to_str()) != Some(&identity.namespace()) {
        return Err("Discovery identity does not match namespace".into());
    }
    let path = root.join("identity.json");
    let bytes = identity.encode()?;
    if path.exists() {
        let mut existing = Vec::new();
        regular_file(&path, false, false)?
            .take(8193)
            .read_to_end(&mut existing)
            .map_err(|e| e.to_string())?;
        if existing != bytes {
            return Err("Saved mission identity differs; original preserved".into());
        }
        return Ok(());
    }
    let temporary = root.join(format!(
        ".identity-{}-{}.tmp",
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
        // Atomic creation without replacing another writer's complete identity.
        match fs::hard_link(&temporary, &path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let mut existing = Vec::new();
                regular_file(&path, false, false)?
                    .take(8193)
                    .read_to_end(&mut existing)
                    .map_err(|e| e.to_string())?;
                if existing != bytes {
                    return Err("Saved mission identity differs; original preserved".into());
                }
            }
            Err(e) => return Err(e.to_string()),
        }
        File::open(&root)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}

#[derive(Default, Debug)]
pub struct Discovery {
    pub names: Vec<String>,
    pub workspaces: Vec<PathBuf>,
    pub skipped: usize,
    pub limited: bool,
}
pub fn discover(state: &Path, workspace: &Path) -> Result<Discovery> {
    scan(state, Some(workspace))
}
pub fn discover_workspaces(state: &Path) -> Result<Discovery> {
    scan(state, None)
}
fn scan(state: &Path, workspace: Option<&Path>) -> Result<Discovery> {
    let root = state.join("rust-tasks-v1");
    for ancestor in root.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("Discovery state ancestors must not be symlinks".into())
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => {}
        }
    }
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Discovery::default()),
        Err(e) => return Err(e.to_string()),
    };
    let mut result = Discovery::default();
    let mut remaining = MAX_BYTES;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_SCAN {
            result.limited = true;
            break;
        }
        let parsed = (|| -> Result<Identity> {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                return Err("Not a namespace directory".into());
            }
            let manifest = entry.path().join("mission.json");
            let identity_path = if fs::symlink_metadata(&manifest).is_ok() {
                manifest
            } else {
                entry.path().join("identity.json")
            };
            let has_identity = fs::symlink_metadata(&identity_path).is_ok();
            let path = if has_identity {
                identity_path
            } else {
                entry.path().join("tasks.json")
            };
            let cap = if has_identity { 8192 } else { 4 * 1024 * 1024 };
            let mut bytes = Vec::new();
            regular_file(&path, false, false)?
                .take((cap.min(remaining) + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > remaining {
                remaining = 0;
                return Err("Discovery byte budget exhausted".into());
            }
            remaining -= bytes.len();
            if bytes.len() > cap {
                return Err("Discovery record exceeds bounds".into());
            }
            let identity: Identity = if has_identity {
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?
            } else {
                // Legacy task identity is a suggestion only; normal loading validates receipts.
                #[derive(Deserialize)]
                struct Legacy {
                    schema_version: u32,
                    workspace: PathBuf,
                    mission: String,
                }
                let legacy: Legacy = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                if !(1..=crate::tasks::SCHEMA_VERSION).contains(&legacy.schema_version) {
                    return Err("Unsupported task version".into());
                }
                Identity {
                    schema_version: 1,
                    workspace: legacy.workspace,
                    mission: legacy.mission,
                }
            };
            if !identity.valid() || entry.file_name().to_str() != Some(&identity.namespace()) {
                return Err("Invalid discovery identity".into());
            }
            Ok(identity)
        })();
        match parsed {
            Ok(identity) if workspace.is_none_or(|path| identity.workspace == path) => {
                result.workspaces.push(identity.workspace);
                if workspace.is_some() {
                    result.names.push(identity.mission);
                }
            }
            Ok(_) => {}
            Err(_) => result.skipped += 1,
        }
        if remaining == 0 {
            result.limited = true;
            break;
        }
    }
    result.workspaces.sort();
    result.workspaces.dedup();
    result.names.sort();
    result.names.dedup();
    Ok(result)
}

// Canonical identity admission is distinct from advisory identity.json discovery.
pub(crate) fn validate_manifest(path: &Path, workspace: &Path, mission: &str) -> Result<()> {
    let mut bytes = Vec::new();
    regular_file(path, false, false)?
        .take(8193)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 8192 {
        return Err("Mission identity exceeds bounds".into());
    }
    let identity: Identity = serde_json::from_slice(&bytes)
        .map_err(|_| "Invalid mission identity; original preserved")?;
    if !identity.valid() || identity.workspace != workspace || identity.mission != mission {
        return Err("Mission identity mismatch".into());
    }
    Ok(())
}
pub(crate) fn create_manifest(path: &Path, workspace: &Path, mission: &str) -> Result<()> {
    let identity = Identity {
        schema_version: 1,
        workspace: workspace.into(),
        mission: mission.into(),
    };
    if !identity.valid() {
        return Err("Invalid mission identity".into());
    }
    let bytes = identity.encode()?;
    let root = path.parent().ok_or("Invalid mission directory")?;
    let temporary = root.join(format!(
        ".mission-{}-{}.tmp",
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
        fs::hard_link(&temporary, path).map_err(|e| {
            format!("Mission creation was not acknowledged: {e}; resume the same name if it exists")
        })?;
        File::open(root).and_then(|f| f.sync_all()).map_err(|e| {
            format!("Mission creation acknowledgment uncertain: {e}; resume the same name")
        })?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escaped_identity_size_is_bounded_before_publication() {
        let identity = Identity {
            schema_version: 1,
            workspace: PathBuf::from(format!("/{}", "\"".repeat(4095))),
            mission: "\"".repeat(120),
        };
        assert!(identity.valid());
        assert!(identity.encode().is_err());
    }
}
