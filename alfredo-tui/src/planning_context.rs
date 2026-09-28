//! Bounded committed repository inputs. No checkout, working-file reads or Git effects.
use crate::worker::git;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub path: String,
    pub blob: String,
    pub content: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryContext {
    pub baseline: String,
    pub paths: Vec<String>,
    pub omitted_paths: usize,
    pub sources: Vec<Source>,
    pub omitted_sources: usize,
}
fn sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn path_valid(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && !path.chars().any(char::is_control)
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}
/// Common secret material and generated/vendor trees are never model inputs.
fn excluded(lower: &str) -> bool {
    lower.split('/').any(|part| {
        matches!(part, ".git" | "node_modules" | "target" | "dist" | "vendor")
            || part.starts_with(".env")
    }) || [".pem", ".key", ".p12", ".pfx"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

/// A committed file named by a check or task text; `content` is Err with the
/// reason it was not read.
#[derive(Debug)]
pub struct NamedSource {
    pub path: String,
    pub content: Result<String, &'static str>,
}

const NAMED_LIMIT: usize = 16;
const NAMED_BYTES: usize = 32 * 1024;

fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\''
                    | '`'
                    | ','
                    | ';'
                    | ':'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '='
            )
    })
    .map(|token| {
        let token = token.trim_end_matches(['.', '!', '?']);
        token.strip_prefix("./").unwrap_or(token)
    })
    .filter(|token| !token.is_empty())
}

/// Tracked regular files at the exact `baseline` commit that the check argv names
/// (first) or that `texts` mention verbatim (then), excluding `exclude`. Reads
/// Git objects only, never working files.
pub async fn named_sources(
    root: &Path,
    baseline: &str,
    check: &[String],
    texts: &[&str],
    exclude: &[String],
) -> Result<Vec<NamedSource>, String> {
    if !sha(baseline) {
        return Err("Reference baseline must be an exact commit identity".into());
    }
    let mut wanted: Vec<&str> = Vec::new();
    for token in check
        .iter()
        .flat_map(|arg| tokens(arg))
        .chain(texts.iter().flat_map(|text| tokens(text)))
    {
        if path_valid(token)
            && !wanted.contains(&token)
            && !exclude.iter().any(|path| path == token)
        {
            wanted.push(token);
        }
    }
    wanted.truncate(64);
    if wanted.is_empty() {
        return Ok(vec![]);
    }
    // Only the named entries: literal pathspecs, no recursion into named directories.
    let mut args = vec!["--literal-pathspecs", "ls-tree", "-l", "-z", baseline, "--"];
    args.extend(wanted.iter().copied());
    let tree = git(root, &args).await?;
    let mut tracked = std::collections::BTreeMap::new();
    for record in tree.split('\0').filter(|record| !record.is_empty()) {
        let (metadata, path) = record.split_once('\t').ok_or("Malformed Git tree entry")?;
        let fields: Vec<_> = metadata.split_whitespace().collect();
        if fields.len() == 4 && matches!(fields[0], "100644" | "100755") && fields[1] == "blob" {
            tracked.insert(path, (fields[2], fields[3]));
        }
    }
    let mut result = Vec::new();
    for path in wanted {
        let Some((blob, size)) = tracked.get(path) else {
            continue;
        };
        if result.len() == NAMED_LIMIT {
            result.push(NamedSource {
                path: path.into(),
                content: Err("reference file limit"),
            });
            continue;
        }
        let content = if excluded(&path.to_lowercase()) {
            Err("excluded as possible secret or generated file")
        } else if size
            .parse::<usize>()
            .map_or(true, |size| size > NAMED_BYTES)
        {
            Err("exceeds 32 KiB")
        } else {
            match git(root, &["cat-file", "blob", blob]).await {
                Ok(text) if !text.contains('\0') => Ok(text),
                Ok(_) => Err("binary"),
                Err(error) if error.contains("non-UTF-8") => Err("not UTF-8 text"),
                Err(error) => return Err(error),
            }
        };
        result.push(NamedSource {
            path: path.into(),
            content,
        });
    }
    Ok(result)
}

impl RepositoryContext {
    pub fn validate(&self) -> Result<(), String> {
        if !sha(&self.baseline)
            || self.paths.len() > 256
            || self.sources.len() > 8
            || self.paths.iter().any(|path| !path_valid(path))
            || self.paths.iter().collect::<BTreeSet<_>>().len() != self.paths.len()
            || self
                .sources
                .iter()
                .map(|source| &source.path)
                .collect::<BTreeSet<_>>()
                .len()
                != self.sources.len()
            || self.sources.iter().any(|source| {
                !path_valid(&source.path)
                    || !sha(&source.blob)
                    || source.content.len() > 8192
                    || source.content.contains('\0')
            })
            || serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 64 * 1024
        {
            return Err("Repository planning context is invalid or exceeds its bounds".into());
        }
        Ok(())
    }
}

pub async fn capture(workspace: &Path, prompt: &str) -> Result<RepositoryContext, String> {
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        capture_inner(workspace, prompt, None),
    )
    .await
    .map_err(|_| "Repository planning context exceeded 30 seconds".to_string())?
}
/// Read only an exact commit, never a mutable ref or the current worktree.
pub async fn capture_at(
    workspace: &Path,
    prompt: &str,
    baseline: &str,
) -> Result<RepositoryContext, String> {
    if !sha(baseline) {
        return Err("Pinned planning baseline must be an exact commit identity".into());
    }
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        capture_inner(workspace, prompt, Some(baseline)),
    )
    .await
    .map_err(|_| "Repository planning context exceeded 30 seconds".to_string())?
}
async fn capture_inner(
    workspace: &Path,
    prompt: &str,
    pinned: Option<&str>,
) -> Result<RepositoryContext, String> {
    let root = git(workspace, &["rev-parse", "--show-toplevel"]).await?;
    if Path::new(root.trim()) != workspace {
        return Err("Planning workspace must be the exact Git root".into());
    }
    let baseline = if let Some(pinned) = pinned {
        if git(workspace, &["cat-file", "-t", pinned]).await?.trim() != "commit" {
            return Err("Pinned planning baseline is not a commit".into());
        }
        pinned.to_string()
    } else {
        git(workspace, &["rev-parse", "HEAD^{commit}"])
            .await?
            .trim()
            .to_string()
    };
    if !sha(&baseline) {
        return Err("Planning needs a committed Git baseline".into());
    }
    let tree = git(workspace, &["ls-tree", "-r", "-l", "-z", &baseline]).await?;
    struct Entry {
        path: String,
        blob: String,
        size: usize,
        score: usize,
    }
    let terms: Vec<_> = prompt
        .split(|c: char| !c.is_alphanumeric())
        .filter(|term| term.len() > 2)
        .map(str::to_lowercase)
        .collect();
    let mut entries = Vec::new();
    let mut total = 0;
    for record in tree.split('\0').filter(|record| !record.is_empty()) {
        total += 1;
        let (metadata, path) = record.split_once('\t').ok_or("Malformed Git tree entry")?;
        let fields: Vec<_> = metadata.split_whitespace().collect();
        if fields.len() != 4 {
            return Err("Malformed Git tree metadata".into());
        }
        if !matches!(fields[0], "100644" | "100755") || fields[1] != "blob" || !path_valid(path) {
            continue;
        }
        let lower = path.to_lowercase();
        if excluded(&lower) {
            continue;
        }
        let base = lower.rsplit('/').next().unwrap_or(&lower);
        let priority = match lower.as_str() {
            "agents.md" => 1000,
            "readme.md" | "context.md" => 900,
            "cargo.toml" | "package.json" | "pyproject.toml" | "makefile" => 800,
            _ => 0,
        };
        let score = priority
            + terms
                .iter()
                .filter(|term| lower.contains(term.as_str()))
                .count()
                .min(10)
                * 20
            + usize::from(base == "agents.md") * 100;
        entries.push(Entry {
            path: path.into(),
            blob: fields[2].into(),
            size: fields[3].parse().map_err(|_| "Invalid Git blob size")?,
            score,
        });
    }
    entries.sort_by(|a, b| b.score.cmp(&a.score).then(a.path.cmp(&b.path)));
    let mut context = RepositoryContext {
        baseline,
        paths: vec![],
        omitted_paths: 0,
        sources: vec![],
        omitted_sources: 0,
    };
    let mut map_bytes = 0;
    for entry in &entries {
        if context.paths.len() < 256 && map_bytes + entry.path.len() <= 16 * 1024 {
            map_bytes += entry.path.len();
            context.paths.push(entry.path.clone());
        }
    }
    context.omitted_paths = total - context.paths.len();
    let mut source_bytes = 0;
    for entry in entries.iter().take(32) {
        if context.sources.len() == 8 || entry.size > 8192 || source_bytes + entry.size > 24 * 1024
        {
            continue;
        }
        let content = match git(workspace, &["cat-file", "blob", &entry.blob]).await {
            Ok(content) => content,
            Err(error) if error.contains("non-UTF-8") => continue,
            Err(error) => return Err(error),
        };
        if content.contains('\0') {
            continue;
        }
        source_bytes += content.len();
        context.sources.push(Source {
            path: entry.path.clone(),
            blob: entry.blob.clone(),
            content,
        });
        if serde_json::to_vec(&context)
            .map_err(|e| e.to_string())?
            .len()
            > 64 * 1024
        {
            context.sources.pop();
            break;
        }
    }
    context.omitted_sources = total - context.sources.len();
    context.validate()?;
    Ok(context)
}
