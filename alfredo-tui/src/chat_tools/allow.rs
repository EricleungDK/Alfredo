//! Always-allow rules (issue #238, ADR 0004): the owner answers `a` on an approval card
//! and later identical actions in this mission run without asking. A rule covers one
//! tool (`edit_file`, `git_commit`, …) or, for `run_command`, one command prefix;
//! rules live in `chat-allow.json` in the mission's State Directory.
//!
//! Strict by design:
//! - Only [`super::Tier::allowlistable`] tools match; `git_push` and push cards never do, even
//!   from a hand-edited file.
//! - A host (escalated) run is never offered `a` and never matches a rule, so every
//!   HOST run asks (issue #311). A rule written before that, with `escalated` set, is
//!   kept only so `/allow` can show it, and it never matches.
//! - A sandboxed rule is a prefix: ANY arguments after it are allowed (the sandbox
//!   bounds the impact). A sandboxed rule never covers a host run.
//! - A run_command invoking `git … push` ([`pushes`]) is never offered or matched.
//! - Rules ignore the call's `cwd`.
//! - Command prefixes match whole whitespace-separated words. A command holding a
//!   shell operator ([`METACHARS`]) matches only a rule that is exactly that command.
//! - An unreadable (malformed or oversized) file is an error naming the path; it matches
//!   nothing (fail closed) and is never overwritten. A missing or empty file is no rules.
//!
//! Matching only replaces the owner's keypress: the card is still previewed (refusals
//! still refuse), the fingerprint is still checked at apply, and the outcome is logged
//! with the rule id.
use super::{approval::Card, approval::CardKind, TOOLS};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Rules file name in the mission's State Directory.
pub const FILE: &str = "chat-allow.json";
/// The largest rules file read.
const MAX_FILE: u64 = 1024 * 1024;
/// Characters that make a command eligible only for an exact-command rule.
pub const METACHARS: &[char] = &[
    ';', '&', '|', '`', '$', '>', '<', '\n', '\r', '\\', '(', ')',
];
/// Most rules kept per mission.
const MAX_RULES: usize = 200;

/// One always-allow rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub id: u64,
    pub tool: String,
    /// run_command only: the command prefix (whole words).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// run_command only. Set only by rules written before issue #311, when a HOST run could
    /// be always-allowed. Such a rule never matches (HOST runs always ask); `/allow` shows
    /// it as inactive.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub escalated: bool,
}

/// Whether `card` may be always-allowed: an allowlistable tool, not a push card, not a
/// run_command that invokes `git … push`, not a HOST (escalated) run (issue #311: a host
/// run always asks, so no rule can skip its card), and not a write_file that creates
/// directories (issue #272: those always ask, so a rule for write_file never covers making
/// directories; the card fingerprint carries `+dirs:` exactly then).
pub fn offered(card: &Card) -> bool {
    card.kind != CardKind::Push
        && !card.escalated()
        && !(card.tool == "write_file" && card.fingerprint.contains("+dirs:"))
        && TOOLS
            .iter()
            .any(|tool| tool.name == card.tool && tool.tier.allowlistable())
        && !command_of(card).is_some_and(pushes)
}

/// Whether a shell command may run `git push`: a `git` word (any path) followed later
/// by a `push` word, quotes ignored (`git -C x push`, `git -c k=v push`). Deliberately
/// broad; a false positive only means the card asks every time.
pub fn pushes(command: &str) -> bool {
    let words: Vec<String> = command
        .split(|c: char| c.is_whitespace() || METACHARS.contains(&c))
        .map(|w| w.trim_matches(|c| c == '"' || c == '\'').to_string())
        .collect();
    words
        .iter()
        .position(|w| w == "git" || w.ends_with("/git"))
        .is_some_and(|at| words[at + 1..].iter().any(|w| w == "push"))
}

/// Whether a rule only matches its exact command: a host (escalated) rule from before
/// issue #311 (it never matches, see [`matches`]), and commands holding shell operators.
fn exact(rule: &Rule) -> bool {
    rule.escalated
        || rule
            .command
            .as_deref()
            .is_some_and(|c| c.contains(METACHARS))
}

/// Whether a rule for `rule` (a command prefix) covers `command`. See the module notes.
pub fn command_matches(rule: &str, command: &str) -> bool {
    let (rule, command) = (rule.trim(), command.trim());
    if rule.is_empty() {
        return false;
    }
    if rule == command {
        return true;
    }
    if rule.contains(METACHARS) || command.contains(METACHARS) {
        return false;
    }
    let words: Vec<&str> = rule.split_whitespace().collect();
    let mut given = command.split_whitespace();
    words.iter().all(|word| given.next() == Some(word))
}

fn command_of(card: &Card) -> Option<&str> {
    card.arguments.get("command").and_then(|c| c.as_str())
}

/// Whether `rule` covers `card`. A card that is not [`offered`] (a HOST run, a push) is
/// never covered, so a rule from before issue #311 never matches a HOST run.
pub fn matches(rule: &Rule, card: &Card) -> bool {
    if !offered(card) || rule.tool != card.tool {
        return false;
    }
    match card.kind {
        CardKind::Shell { escalated } => {
            !escalated
                && !rule.escalated
                && match (&rule.command, command_of(card)) {
                    (Some(text), Some(command)) if exact(rule) => text.trim() == command.trim(),
                    (Some(text), Some(command)) => command_matches(text, command),
                    _ => false,
                }
        }
        _ => rule.command.is_none() && !rule.escalated,
    }
}

/// The rule `a` would add for `card` (id 0), or why none can.
pub fn rule_for(card: &Card) -> Result<Rule, String> {
    if !offered(card) {
        return Err(format!("{} can never be always-allowed", card.tool));
    }
    Ok(match card.kind {
        CardKind::Shell { .. } => {
            let raw = command_of(card).ok_or("run_command has no command")?;
            // Checked before trimming: `ls\r` must not pass as `ls`.
            hint_command(raw)?;
            let command = Some(raw.trim())
                .filter(|c| !c.is_empty())
                .ok_or("run_command has no command")?
                .to_string();
            Rule {
                id: 0,
                tool: card.tool.clone(),
                command: Some(command),
                escalated: false,
            }
        }
        _ => Rule {
            id: 0,
            tool: card.tool.clone(),
            command: None,
            escalated: false,
        },
    })
}

/// Most characters of a command, as shown, that the `a` hint carries (issue #409); a longer
/// command is not offered.
pub const HINT_COMMAND_MAX: usize = 80;

/// One `$ ` line of a `run_command` card as drawn: invisible characters as `<U+XXXX>`, a
/// literal `↵` as `<U+21B5>`, a tab as `→` plus padding (stops counted from the start of
/// the line, `$ ` included) and a literal `→` as `<U+2192>`. The card and
/// [`show_command`] both use it, so they cannot drift (issue #409).
pub fn show_line(line: &str) -> String {
    let visible = crate::dashboard::safe_visible(line).replace('\u{21B5}', "<U+21B5>");
    crate::dashboard::show_tabs(&visible)
}

/// A command as its card's `$` lines show it, the lines joined by `↵`: each line goes
/// through [`show_line`] after its `$ ` prefix. Control characters other than tab and
/// newline, which the card would drop, show as `<U+XXXX>` here, so `/allow` deletes
/// nothing; such a command is never offered ([`hint_command`]). The hint and `/allow`
/// share this (issue #409).
pub fn show_command(command: &str) -> String {
    command
        .split('\n')
        .map(|line| {
            let spelled: String = line
                .chars()
                .map(|c| {
                    if c.is_control() && c != '\t' {
                        format!("<U+{:04X}>", c as u32)
                    } else {
                        c.to_string()
                    }
                })
                .collect();
            show_line(&format!("$ {spelled}"))["$ ".len()..].to_string()
        })
        .collect::<Vec<_>>()
        .join("↵")
}

/// The command as the hint shows it, or why the hint cannot show all of it: control
/// characters other than tab and newline (the card would drop them) and text that looks
/// like one of the `<U+XXXX>` escapes are refused, and so is a command that shows as more
/// than [`HINT_COMMAND_MAX`] characters.
fn hint_command(command: &str) -> Result<String, String> {
    if command
        .chars()
        .any(|c| c.is_control() && c != '\t' && c != '\n')
    {
        return Err("has control characters it cannot show".into());
    }
    if command.contains("<U+") {
        return Err("contains text that looks like an escape".into());
    }
    let shown = show_command(command.trim());
    if shown.chars().count() > HINT_COMMAND_MAX {
        return Err(format!(
            "too long to always-allow (over {HINT_COMMAND_MAX} characters as shown)"
        ));
    }
    Ok(shown)
}

/// Why `a` is not offered on `card`, for the card's key line: a HOST run always asks
/// (issue #311), and a command the hint cannot show in full is refused (#409).
pub fn unavailable_reason(card: &Card) -> Option<String> {
    if card.escalated() {
        return Some("HOST runs always ask".into());
    }
    if !offered(card) {
        return None;
    }
    rule_for(card).err()
}

/// Whether `a` is available on `card`: the rule can be added and the hint shows it in full.
pub fn available(card: &Card) -> bool {
    offer(card).is_some()
}

/// One line naming what a rule covers, in full (the view wraps it; issue #320).
pub fn describe(rule: &Rule) -> String {
    let clean = |text: &str| -> String {
        text.chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect()
    };
    match &rule.command {
        Some(command) => format!(
            "#{} {} `{}`{}{}",
            rule.id,
            clean(&rule.tool),
            show_command(command),
            if exact(rule) {
                " (exactly)"
            } else {
                " (ANY arguments after it, sandboxed only)"
            },
            if rule.escalated {
                " · on HOST (escalated): inactive, HOST runs always ask"
            } else {
                " · sandboxed"
            }
        ),
        None => format!(
            "#{} {} (every call){}",
            rule.id,
            clean(&rule.tool),
            if rule.escalated {
                " · on HOST (escalated): inactive, HOST runs always ask"
            } else {
                ""
            }
        ),
    }
}

/// Card hint for `a`, or `None` when the card cannot be always-allowed.
pub fn offer(card: &Card) -> Option<String> {
    let rule = rule_for(card).ok()?;
    Some(match &rule.command {
        None => format!("a always allow {} in this mission", rule.tool),
        Some(command) => format!(
            "a always allow `{}`{}",
            hint_command(command).ok()?,
            if exact(&rule) {
                " exactly"
            } else {
                " with ANY arguments after it, sandboxed only"
            }
        ),
    })
}

/// The rules of one mission.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    path: PathBuf,
}

impl Rules {
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join(FILE),
        }
    }

    /// The rules beside the mission's task store.
    pub fn for_store(store: &crate::tasks::TaskStore) -> Result<Self, String> {
        Ok(Self::new(&store.conversation_directory()?))
    }

    /// Rules, oldest first. A missing or empty (0 bytes) file is no rules. A file that is
    /// present but unreadable (not valid rules JSON, over 1 MiB, not a regular file, an I/O
    /// error) is an error naming the path; it is never treated as "no rules".
    pub fn list(&self) -> Result<Vec<Rule>, String> {
        use std::io::Read;
        let unreadable = |problem: &str| {
            let path = self.path.display();
            format!(
                "Always-allow rules file is unreadable: {problem} ({path}); original preserved, no rule applies until it is fixed. Move it aside (mv \"{path}\" \"{path}.bad\") to start with no rules"
            )
        };
        match std::fs::metadata(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(unreadable(&error.to_string())),
            Ok(meta) if !meta.is_file() => return Err(unreadable("not a regular file")),
            Ok(_) => {}
        }
        let mut bytes = Vec::new();
        std::fs::File::open(&self.path)
            .and_then(|file| file.take(MAX_FILE + 1).read_to_end(&mut bytes))
            .map_err(|error| unreadable(&error.to_string()))?;
        if bytes.len() as u64 > MAX_FILE {
            return Err(unreadable("file exceeds 1 MiB"));
        }
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        serde_json::from_slice(&bytes).map_err(|_| unreadable("malformed JSON"))
    }

    /// The first rule covering `card`. An unreadable file matches nothing (fail closed).
    pub fn matching(&self, card: &Card) -> Option<Rule> {
        self.list()
            .ok()?
            .into_iter()
            .find(|rule| matches(rule, card))
    }

    /// Add the rule for `card` (an equal rule is reused).
    pub fn add_for(&self, card: &Card) -> Result<Rule, String> {
        let wanted = rule_for(card)?;
        self.update(|rules| {
            if let Some(rule) = rules.iter().find(|r| {
                r.tool == wanted.tool
                    && r.command == wanted.command
                    && r.escalated == wanted.escalated
            }) {
                return Ok(rule.clone());
            }
            if rules.len() >= MAX_RULES {
                return Err(format!("at most {MAX_RULES} rules; /allow clear some"));
            }
            let rule = Rule {
                id: rules.iter().map(|r| r.id).max().unwrap_or(0) + 1,
                ..wanted
            };
            rules.push(rule.clone());
            Ok(rule)
        })
    }

    /// Remove rule `id`.
    pub fn remove(&self, id: u64) -> Result<Rule, String> {
        self.update(|rules| {
            let index = rules
                .iter()
                .position(|r| r.id == id)
                .ok_or_else(|| format!("no rule #{id}; /allow lists them"))?;
            Ok(rules.remove(index))
        })
    }

    /// Remove every rule; returns how many there were.
    pub fn clear(&self) -> Result<usize, String> {
        self.update(|rules| Ok(std::mem::take(rules).len()))
    }

    /// Read, change and atomically rewrite the file under a lock.
    fn update<T>(
        &self,
        change: impl FnOnce(&mut Vec<Rule>) -> Result<T, String>,
    ) -> Result<T, String> {
        let failed = |e: std::io::Error| format!("rules not saved: {e}");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.path.with_extension("lock"))
            .map_err(failed)?;
        lock.lock().map_err(failed)?;
        let mut rules = self.list()?;
        let result = change(&mut rules)?;
        let text = serde_json::to_string_pretty(&rules).map_err(|e| e.to_string())?;
        let temporary = self
            .path
            .with_extension(format!("json.{}.tmp", std::process::id()));
        std::fs::write(&temporary, text).map_err(failed)?;
        std::fs::rename(&temporary, &self.path).map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            failed(e)
        })?;
        Ok(result)
    }
}

/// `/allow [clear [ID]]`: `args` is the text after `/allow`. `Ok` is the notice; the rule
/// list is several lines (a heading, then one row per rule).
pub fn slash(rules: &Rules, args: &str) -> Result<String, String> {
    let words: Vec<&str> = args.split_whitespace().collect();
    match words.as_slice() {
        [] => {
            let list = rules.list()?;
            if list.is_empty() {
                return Ok("No always-allow rules · press a on an approval card to add one".into());
            }
            // One row per rule (issue #320); the terminal shows the rows wrapped.
            Ok(format!(
                "Always-allow rules · /allow clear [ID] removes\n{}",
                list.iter().map(describe).collect::<Vec<_>>().join("\n")
            ))
        }
        ["clear"] => rules
            .clear()
            .map(|count| format!("Cleared {count} always-allow rule(s); every action asks again")),
        ["clear", id] => {
            let id = id
                .trim_start_matches('#')
                .parse::<u64>()
                .map_err(|_| "Usage: /allow clear [ID]".to_string())?;
            rules
                .remove(id)
                .map(|rule| format!("Removed {}", describe(&rule)))
        }
        _ => Err("Usage: /allow · /allow clear · /allow clear ID".into()),
    }
}
