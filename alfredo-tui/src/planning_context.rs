//! Bounded committed repository inputs. No checkout, working-file reads or Git effects.
use crate::worker::{git, git_records};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{BTreeSet, BinaryHeap},
    path::Path,
};

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
    /// True when the tree had more than `MAX_TREE_RECORDS` entries: counting
    /// stopped there, so the omitted counts are lower bounds.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub listing_truncated: bool,
}
fn sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
pub(crate) fn path_valid(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && !path.chars().any(char::is_control)
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}
/// Secret material and Git metadata (issues #312 and the original list). `path` is a
/// `/`-separated path in any case; every rule is tested on each lowercased path component,
/// so a directory such as `.aws/` or `.ssh/` excludes everything under it.
///
/// - Directories: `.git`, `.ssh`, `.gnupg`, `.aws`, `.kube`.
/// - Names: `.env*`, `.npmrc`, `.netrc`, `.pgpass`, `.git-credentials`, `secrets.yml`,
///   `secrets.yaml`.
/// - Name prefixes: `id_rsa`, `id_ed25519`, `id_ecdsa`, `id_dsa` (private keys and
///   their `.pub`/backup siblings; a name that merely starts so is refused too), and
///   `credentials` with a `.json` ending (`credentials*.json`).
/// - Suffixes: `.pem`, `.key`, `.p12`, `.pfx`, `.env`, `.keystore`, `.jks`, `.tfvars`,
///   `.tfvars.json`.
///
/// Bare `credentials` and `secrets.*` names other than the two above stay readable.
pub(crate) fn secret(path: &str) -> bool {
    path.split('/').any(|raw| {
        let lower = raw.to_lowercase();
        let part = lower.as_str();
        matches!(
            part,
            ".git"
                | ".ssh"
                | ".gnupg"
                | ".aws"
                | ".kube"
                | ".npmrc"
                | ".netrc"
                | ".pgpass"
                | ".git-credentials"
                | "secrets.yml"
                | "secrets.yaml"
        ) || part.starts_with(".env")
            || ["id_rsa", "id_ed25519", "id_ecdsa", "id_dsa"]
                .iter()
                .any(|prefix| part.starts_with(prefix))
            || (part.starts_with("credentials")
                && Path::new(part).extension().is_some_and(|ext| ext == "json"))
            || [
                ".pem",
                ".key",
                ".p12",
                ".pfx",
                ".env",
                ".keystore",
                ".jks",
                ".tfvars",
                ".tfvars.json",
            ]
            .iter()
            .any(|suffix| part.ends_with(suffix))
    })
}

/// Generated and vendor trees: `node_modules`, `target`, `dist`, `vendor`.
pub(crate) fn generated(lower: &str) -> bool {
    lower
        .split('/')
        .any(|part| matches!(part, "node_modules" | "target" | "dist" | "vendor"))
}

/// Common secret material and generated/vendor trees are never model inputs.
pub(crate) fn excluded(lower: &str) -> bool {
    secret(lower) || generated(lower)
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
/// Most tree entries counted for one context. A larger tree is cut here and the
/// context says so (`listing_truncated`) instead of planning failing.
pub const MAX_TREE_RECORDS: usize = 1_000_000;
/// Best-ranked candidates kept while streaming the tree (256 map paths, with
/// slack for the 16 KiB map budget, and 32 source candidates).
const POOL: usize = 1024;

struct Entry {
    path: String,
    blob: String,
    size: usize,
    score: usize,
}
// Greater means worse (lower score, then later path) so the max-heap root is
// the entry to evict.
impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .score
            .cmp(&self.score)
            .then_with(|| self.path.cmp(&other.path))
    }
}
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Entry {}

async fn capture_inner(
    workspace: &Path,
    prompt: &str,
    pinned: Option<&str>,
) -> Result<RepositoryContext, String> {
    capture_with_record_limit(workspace, prompt, pinned, MAX_TREE_RECORDS).await
}

#[doc(hidden)]
pub async fn capture_with_record_limit(
    workspace: &Path,
    prompt: &str,
    pinned: Option<&str>,
    max_records: usize,
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
        if git(
            workspace,
            &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
        )
        .await
        .is_err()
        {
            return Err(crate::worker::NO_COMMITS.into());
        }
        git(workspace, &["rev-parse", "HEAD^{commit}"])
            .await?
            .trim()
            .to_string()
    };
    if !sha(&baseline) {
        return Err("Planning needs a committed Git baseline".into());
    }
    let terms: Vec<_> = prompt
        .split(|c: char| !c.is_alphanumeric())
        .filter(|term| term.len() > 2)
        .map(str::to_lowercase)
        .collect();
    let mut heap: BinaryHeap<Entry> = BinaryHeap::new();
    let mut total = 0;
    let mut listing_truncated = false;
    git_records(
        workspace,
        &["ls-tree", "-r", "-l", "-z", &baseline],
        |record| {
            if total == max_records {
                listing_truncated = true;
                return Ok(false);
            }
            total += 1;
            let Ok(record) = std::str::from_utf8(record) else {
                return Ok(true); // a non-UTF-8 path can never be listed
            };
            let (metadata, path) = record.split_once('\t').ok_or("Malformed Git tree entry")?;
            let fields: Vec<_> = metadata.split_whitespace().collect();
            if fields.len() != 4 {
                return Err("Malformed Git tree metadata".into());
            }
            if !matches!(fields[0], "100644" | "100755") || fields[1] != "blob" || !path_valid(path)
            {
                return Ok(true);
            }
            let lower = path.to_lowercase();
            if excluded(&lower) {
                return Ok(true);
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
            let entry = Entry {
                path: path.into(),
                blob: fields[2].into(),
                size: fields[3].parse().map_err(|_| "Invalid Git blob size")?,
                score,
            };
            if heap.len() == POOL {
                if heap.peek().is_some_and(|worst| entry >= *worst) {
                    return Ok(true);
                }
                heap.pop();
            }
            heap.push(entry);
            Ok(true)
        },
    )
    .await?;
    let entries = heap.into_sorted_vec();
    let mut context = RepositoryContext {
        baseline,
        paths: vec![],
        omitted_paths: 0,
        sources: vec![],
        omitted_sources: 0,
        listing_truncated,
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

#[cfg(test)]
mod tests {
    use super::{excluded, secret};

    /// Issue #312: credential files that must never be model inputs, in the forms a
    /// repository commonly holds them.
    #[test]
    fn secret_covers_common_credential_files_and_directories() {
        for path in [
            ".env",
            ".env.local",
            "prod.env",
            "config/a.env",
            ".git/HEAD",
            "key.pem",
            "deploy/server.key",
            "client.p12",
            "client.pfx",
            ".npmrc",
            "web/.npmrc",
            ".netrc",
            ".pgpass",
            ".git-credentials",
            "id_rsa",
            "id_rsa.pub",
            "keys/id_ed25519",
            "id_ecdsa",
            "id_dsa",
            ".ssh/config",
            "home/.ssh/known_hosts",
            ".gnupg/secring.gpg",
            ".aws/credentials",
            ".aws/config",
            "infra/.kube/config",
            "credentials.json",
            "config/credentials-prod.json",
            "app.keystore",
            "release/app.jks",
            "terraform.tfvars",
            "envs/prod.tfvars.json",
            "secrets.yml",
            "config/secrets.yaml",
        ] {
            assert!(secret(path), "{path} must be a secret");
            assert!(excluded(path), "{path} must be excluded");
        }
    }

    #[test]
    fn secret_matches_case_insensitively() {
        for path in [
            ".NPMRC",
            "Prod.ENV",
            "CONFIG/.AWS/Credentials",
            "Id_RSA",
            "A/.SSH/x",
            "Secrets.YAML",
            "Credentials-PROD.JSON",
            "App.JKS",
        ] {
            assert!(secret(path), "{path} must be a secret");
            assert!(secret(&path.to_lowercase()), "{path} must be a secret");
        }
    }

    /// Near misses that stay readable: names that only share a letter sequence
    /// with a credential pattern, and documentation about them.
    #[test]
    fn secret_leaves_near_miss_names_alone() {
        for path in [
            "src/environment.rs",
            "src/envelope.rs",
            "docs/keys.md",
            "src/keyboard.rs",
            "src/secrets.rs",
            "docs/secrets.md",
            "notes/secrets.yml.md",
            "src/credentials.rs",
            "credentials.md",
            "docs/credentials.json.txt",
            "npmrc.md",
            "docs/netrc-notes.md",
            "src/ssh_client.rs",
            "docs/ssh-guide.md",
            "aws/readme.md",
            "src/kubernetes.rs",
            ".sshrc",
            "rsa_parser.rs",
            "tfvars.md",
            "config/app.toml",
        ] {
            assert!(!secret(path), "{path} must stay readable");
        }
    }

    #[test]
    fn generated_trees_stay_excluded_and_unchanged() {
        assert!(excluded("node_modules/x.js"));
        assert!(excluded("pkg/vendor/lib.go"));
        assert!(!secret("pkg/vendor/lib.go"));
        assert!(!secret("src/main.rs"));
    }
}
