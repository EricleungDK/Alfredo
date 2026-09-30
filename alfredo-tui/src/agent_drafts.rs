//! Unsent agent-view drafts kept across restarts: one small file per
//! conversation beside the owner instruction state. An empty draft has no
//! entry and no drafts means no file. A file that cannot be read back is moved
//! aside (never overwritten) and the drafts start empty; nothing here blocks
//! startup or authorizes work.
use crate::agent_view::Target;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const VERSION: u32 = 1;
const MAX_STATE: usize = 256 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    /// Task family repair root; 0 for the architect.
    root: u64,
    text: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    drafts: Vec<Entry>,
}

/// Drafts of one agent target each, as typed and not yet sent.
pub type Drafts = BTreeMap<Target, String>;

/// The draft file of one conversation set in a mission directory.
pub fn state_path(directory: &Path, conversation: &str) -> PathBuf {
    directory.join(format!(
        "agent-drafts-{:x}.json",
        Sha256::digest(conversation.as_bytes())
    ))
}

fn root(target: Target) -> u64 {
    match target {
        Target::Task(root) => root,
        Target::Architect => 0,
    }
}

fn target(root: u64) -> Target {
    match root {
        0 => Target::Architect,
        root => Target::Task(root),
    }
}

/// Where the drafts live and what was last written, to skip unchanged writes.
pub struct Store {
    path: PathBuf,
    saved: Drafts,
    failed: bool,
}

impl Store {
    /// Load this conversation's drafts. Unreadable state yields no drafts and a
    /// notice; invalid content is first moved aside so later saves keep it.
    pub fn open(directory: &Path, conversation: &str) -> (Self, Drafts, Option<String>) {
        let path = state_path(directory, conversation);
        let (drafts, notice) = match read(&path) {
            Ok(drafts) => (drafts, None),
            Err(Bad::Invalid(why)) => {
                let aside = path.with_extension("json.corrupt");
                let kept = match fs::rename(&path, &aside) {
                    Ok(()) => format!("kept as {}", file_name(&aside)),
                    Err(error) => format!("could not move it aside ({error})"),
                };
                (
                    Drafts::new(),
                    Some(format!(
                        "Agent drafts file is invalid ({why}); {kept}, drafts start empty"
                    )),
                )
            }
            Err(Bad::Io(error)) => (
                Drafts::new(),
                Some(format!(
                    "Cannot read agent drafts ({error}); drafts start empty"
                )),
            ),
        };
        let store = Self {
            path,
            saved: drafts.clone(),
            failed: false,
        };
        (store, drafts, notice)
    }

    /// Write `drafts` when they differ from what was last written. Returns the
    /// error the first time writing starts failing; the drafts stay in memory.
    pub fn sync(&mut self, drafts: &Drafts) -> Option<String> {
        if *drafts == self.saved {
            return None;
        }
        match write(&self.path, drafts) {
            Ok(()) => {
                self.saved = drafts.clone();
                self.failed = false;
                None
            }
            Err(error) => {
                let first = !self.failed;
                self.failed = true;
                first.then(|| format!("Agent draft save failed: {error}; drafts kept until quit"))
            }
        }
    }
}

enum Bad {
    Invalid(String),
    Io(std::io::Error),
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn read(path: &Path) -> Result<Drafts, Bad> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Drafts::new()),
        Err(error) => return Err(Bad::Io(error)),
    };
    if bytes.len() > MAX_STATE {
        return Err(Bad::Invalid("over its size bound".into()));
    }
    let saved: Saved = serde_json::from_slice(&bytes).map_err(|e| Bad::Invalid(e.to_string()))?;
    if saved.version != VERSION {
        return Err(Bad::Invalid(format!(
            "unsupported version {}",
            saved.version
        )));
    }
    Ok(saved
        .drafts
        .into_iter()
        .filter(|entry| !entry.text.is_empty())
        .map(|entry| (target(entry.root), entry.text))
        .collect())
}

fn write(path: &Path, drafts: &Drafts) -> Result<(), String> {
    if drafts.is_empty() {
        return match fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.to_string()),
            _ => Ok(()),
        };
    }
    let saved = Saved {
        version: VERSION,
        drafts: drafts
            .iter()
            .map(|(target, text)| Entry {
                root: root(*target),
                text: text.clone(),
            })
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&saved).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_STATE {
        return Err("drafts exceed the size bound".into());
    }
    let directory = path.parent().ok_or("Missing state directory")?;
    let temporary = directory.join(format!(".agent-drafts-{}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    File::open(directory)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| e.to_string())
}
