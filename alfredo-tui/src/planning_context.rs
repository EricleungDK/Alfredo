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
/// Secret material and Git metadata (issues #312, #471 and #523). `path` is a
/// `/`-separated path in any case; every rule is tested on each lowercased path component,
/// so a directory such as `.aws/` or `.ssh/` excludes everything under it.
///
/// Before matching, `.` and empty components are dropped: `.docker//config.json` is read
/// as `.docker/config.json`. A `..` is also resolved lexically, and the path is a secret
/// when its raw reading or its resolved reading is one, so `a/.docker/x/../config.json`
/// is refused and no `..` turns a secret into a plain path.
///
/// - Directories: `.git`, `.ssh`, `.gnupg`, `.aws`, `.kube`, `.azure`, `.password-store`
///   (everything under them), and `gh` under `.config` (`.config/gh`), and `hub` under
///   `.config` (`.config/hub`).
/// - Directory-scoped files: `.docker/config.json`, `.cargo/credentials` and
///   `.cargo/credentials.toml`, `.gem/credentials`, `.mc/config.json`, `.m2/settings.xml`,
///   `.pip/pip.conf`, `.config/rclone/rclone.conf`, `.config/gcloud/credentials.db`,
///   `.config/gcloud/access_tokens.db`, `.config/git/credentials`, `.config/pip/pip.conf`
///   and `.config/netrc` (the `.config` directory is required: `config/git/credentials`
///   without the dot stays readable, like `pip/pip.conf` next to `.pip/pip.conf`).
///   Only those names: `.docker/daemon.json`, `.cargo/config.toml` and `.m2/` files other than
///   `settings.xml` stay readable.
/// - Names: `.env*` (`secret` refuses all of them; [`read_secret`] lets reads through the
///   exact final names `.env.example`, `.env.sample` and `.env.template`, which hold no
///   secrets; a directory of that name exempts nothing), `.npmrc`, `.netrc`, `_netrc`,
///   `.pgpass`, `.git-credentials`, `.pypirc`,
///   `.htpasswd`, `.vault-token`, `.s3cfg`, `.boto`, `.dockercfg`, `.my.cnf`, `.terraformrc`,
///   `terraform.rc`, `auth.json` (Composer), `.bash_history`, `.zsh_history`,
///   `.zhistory`, `.mysql_history`, `.psql_history`, `.python_history`,
///   `.node_repl_history`, `.sqlite_history`, `.authinfo`, `.sentryclirc`, `gradle.properties`,
///   `.rclone.conf`, `.yarnrc.yml`, `.dev.vars`, `service_account.json`, `kubeconfig`,
///   `kubeconfig.yaml`, `kubeconfig.yml`, `application_default_credentials.json`,
///   `secrets.y*ml` (starts `secrets.y`, ends `ml`).
/// - Name prefixes: `id_rsa`, `id_ed25519`, `id_ecdsa`, `id_dsa` (private keys and
///   their `.pub`/backup siblings; a name that merely starts so is refused too),
///   `credentials` with a `.json` ending (`credentials*.json`), `service-account` and
///   `client_secret` with a `.json` ending (`service-account*.json`, `client_secret*.json`),
///   `firebase-adminsdk-` with a `.json` ending (`firebase-adminsdk-*.json`).
/// - Globs: `ssh_host_*_key` (starts `ssh_host_`, ends `_key`), `private*.asc` and
///   `secret*.asc` (starts `private` or `secret`, ends `.asc`). Other `.asc` files, such as
///   `docs/*.asc`, stay readable.
/// - Suffixes: `.pem`, `.key`, `.p12`, `.pfx`, `.env`, `.keystore`, `.jks`, `.tfvars`,
///   `.tfvars.json`, `.tfstate`, `.tfstate.backup`, `.p8`, `.ppk`, `.gpg`, `.kdbx`.
/// - Timestamped Terraform backups: a name containing `.tfstate.` and ending `.backup`
///   (`terraform.tfstate.1700000000.backup`).
///
/// Bare `credentials` and `secrets.*` names other than the two above stay readable.
/// Directory-scoped rules look at the component directly above the name, or at the whole
/// directory chain for the longer paths, so a nested `sub/.docker/config.json` is refused too.
pub(crate) fn secret(path: &str) -> bool {
    secret_by(path, false)
}

/// [`secret`] for the read consumers only (read tools, `@path`, planner context, diff
/// display): the exact final names `.env.example`, `.env.sample` and `.env.template` are
/// readable. Writes and commits keep using [`secret`] and refuse them.
pub(crate) fn read_secret(path: &str) -> bool {
    secret_by(path, true)
}

fn secret_by(path: &str, templates_readable: bool) -> bool {
    let lower = path.to_lowercase();
    let raw: Vec<&str> = lower
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    secret_components(&raw, templates_readable)
        || secret_components(&resolve_parent_refs(&raw), templates_readable)
}

/// `parts` with each `..` resolved against the component before it. A `..` with nothing
/// to resolve (at the start, or after another `..`) is kept.
fn resolve_parent_refs<'a>(parts: &[&'a str]) -> Vec<&'a str> {
    let mut resolved: Vec<&str> = Vec::with_capacity(parts.len());
    for &part in parts {
        if part == ".." && resolved.last().is_some_and(|last| *last != "..") {
            resolved.pop();
        } else {
            resolved.push(part);
        }
    }
    resolved
}

/// The `.env*` names that reads may open: secret-free templates (see [`read_secret`]).
const ENV_TEMPLATES: [&str; 3] = [".env.example", ".env.sample", ".env.template"];

/// Whether one of the normalised, lowercased `parts` is a secret (see [`secret`]).
fn secret_components(parts: &[&str], templates_readable: bool) -> bool {
    parts.iter().enumerate().any(|(index, &part)| {
        // The component directly above this one, for the rules scoped to a directory.
        let parent = index.checked_sub(1).map(|up| parts[up]);
        // The components from the root down to this one, for the rules scoped to a path.
        let chain = &parts[..=index];
        // A committed, secret-free env template: only as the final component, so a
        // directory of that name does not exempt what is inside it.
        let env_template =
            templates_readable && index + 1 == parts.len() && ENV_TEMPLATES.contains(&part);
        let json = Path::new(part).extension().is_some_and(|ext| ext == "json");
        matches!(
            part,
            ".git"
                | ".ssh"
                | ".gnupg"
                | ".aws"
                | ".kube"
                | ".azure"
                | ".password-store"
                | ".npmrc"
                | ".netrc"
                | ".pgpass"
                | ".git-credentials"
                | ".pypirc"
                | ".htpasswd"
                | ".vault-token"
                | ".s3cfg"
                | ".boto"
                | ".dockercfg"
                | "kubeconfig"
                | "kubeconfig.yaml"
                | "kubeconfig.yml"
                | "application_default_credentials.json"
                | "_netrc"
                | ".my.cnf"
                | ".terraformrc"
                | "terraform.rc"
                | "auth.json"
                | ".bash_history"
                | ".zsh_history"
                | ".zhistory"
                | ".mysql_history"
                | ".psql_history"
                | ".python_history"
                | ".node_repl_history"
                | ".sqlite_history"
                | ".authinfo"
                | ".sentryclirc"
                | "gradle.properties"
                | ".rclone.conf"
                | ".yarnrc.yml"
                | ".dev.vars"
                | "service_account.json"
        ) || (parent == Some(".config") && matches!(part, "gh" | "hub"))
            || (parent == Some(".docker") && part == "config.json")
            || (parent == Some(".cargo") && matches!(part, "credentials" | "credentials.toml"))
            || (parent == Some(".gem") && part == "credentials")
            || chain.ends_with(&[".mc", "config.json"])
            || chain.ends_with(&[".m2", "settings.xml"])
            || chain.ends_with(&[".pip", "pip.conf"])
            || chain.ends_with(&[".config", "rclone", "rclone.conf"])
            || chain.ends_with(&[".config", "git", "credentials"])
            || chain.ends_with(&[".config", "pip", "pip.conf"])
            || chain.ends_with(&[".config", "netrc"])
            || chain.ends_with(&[".config", "gcloud", "credentials.db"])
            || chain.ends_with(&[".config", "gcloud", "access_tokens.db"])
            || (part.contains(".tfstate.") && part.ends_with(".backup"))
            || (part.starts_with(".env") && !env_template)
            || (part.starts_with("secrets.y") && part.ends_with("ml"))
            || ["id_rsa", "id_ed25519", "id_ecdsa", "id_dsa"]
                .iter()
                .any(|prefix| part.starts_with(prefix))
            || ([
                "credentials",
                "service-account",
                "client_secret",
                "firebase-adminsdk-",
            ]
            .iter()
            .any(|prefix| part.starts_with(prefix))
                && json)
            || (part.starts_with("ssh_host_") && part.ends_with("_key"))
            || ((part.starts_with("private") || part.starts_with("secret"))
                && part.ends_with(".asc"))
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
                ".tfstate",
                ".tfstate.backup",
                ".p8",
                ".ppk",
                ".gpg",
                ".kdbx",
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

/// [`excluded`] for reads: the env templates of [`read_secret`] are not excluded.
pub(crate) fn read_excluded(lower: &str) -> bool {
    read_secret(lower) || generated(lower)
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
        let content = if read_excluded(&path.to_lowercase()) {
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
            if read_excluded(&lower) {
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
    use super::{excluded, read_excluded, read_secret, secret};

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
            "secrets.yaml.yml",
            "secrets.y-prod.yml",
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
            "Config/SECRETS.Y-PROD.YML",
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

    /// Issue #471: credential files outside the first list (registry, Terraform,
    /// cloud and vault tokens, service-account keys), in nested and case-varied forms.
    #[test]
    fn secret_covers_issue_471_credential_files() {
        for path in [
            ".pypirc",
            "home/.PyPIRC",
            ".htpasswd",
            ".vault-token",
            ".s3cfg",
            ".boto",
            ".dockercfg",
            "kubeconfig",
            "deploy/Kubeconfig",
            "kubeconfig.yaml",
            "deploy/kubeconfig.yml",
            "application_default_credentials.json",
            "gcloud/application_default_credentials.json",
            "service-account.json",
            "keys/service-account-prod.json",
            "client_secret.json",
            "client_secret_123.json",
            "AuthKey_X.p8",
            "key.ppk",
            "backup/x.gpg",
            "terraform.tfstate",
            "terraform.tfstate.backup",
            "terraform.tfstate.1700000000.backup",
            "infra/terraform.tfstate.42.BACKUP",
            "infra/prod.tfstate",
            "Terraform.TFState",
            ".docker/config.json",
            "sub/.docker/config.json",
            ".DOCKER/CONFIG.JSON",
            ".cargo/credentials",
            ".cargo/credentials.toml",
            "sub/.cargo/Credentials.TOML",
            ".gem/credentials",
            ".config/gh/hosts.yml",
            ".config/gh/config.yml",
            "home/.config/gh/nested/x.txt",
            ".azure/accessTokens.json",
            ".azure/azureProfile.json",
            ".password-store/x.gpg",
            ".password-store/notes.txt",
        ] {
            assert!(secret(path), "{path} must be a secret");
            assert!(excluded(path), "{path} must be excluded");
        }
    }

    /// Issue #471 near misses: the directory-scoped rules only cover the named files
    /// (`.docker/config.json`, `.cargo/credentials*`, `.gem/credentials`), and the name
    /// rules need an exact name, suffix or `.json` ending.
    #[test]
    fn secret_leaves_issue_471_near_misses_alone() {
        for path in [
            "src/state.rs",
            "terraform.md",
            "docs/terraform.md",
            "terraform.tfstate.md",
            "terraform.tfstate.backup.md",
            "docs/terraform.tfstate.notes.txt",
            "config.json",
            "docs/config.json",
            ".docker/daemon.json",
            ".docker/config.json.md",
            "docs/.docker/readme.md",
            ".cargo/config.toml",
            ".cargo/config",
            "src/key.rs",
            "docs/kubeconfig.md",
            "kubeconfig.md",
            "service-account.md",
            "client_secret.md",
            "src/boto.rs",
            "AuthKey_X.p8.md",
            "docs/gpg.md",
            ".gem/config",
            ".config/ghost.md",
            ".config/gh.md",
            ".azure.md",
            "docs/password-store.md",
            "gcloud/config.json",
        ] {
            assert!(!secret(path), "{path} must stay readable");
        }
    }

    /// Issue #523: common credential files that #471 did not list (package and database
    /// clients, Terraform and Maven settings, cloud CLIs, KeePass, shell history), with
    /// case variants. Directory-scoped names need their directory chain.
    #[test]
    fn secret_covers_issue_523_credential_files() {
        let mut missed = vec![];
        for path in [
            "_netrc",
            "home/_NETRC",
            ".my.cnf",
            ".My.Cnf",
            "db/.my.cnf",
            ".terraformrc",
            "terraform.rc",
            "Terraform.RC",
            "app/auth.json",
            "auth.json",
            "secrets.kdbx",
            "vault/Passwords.KDBX",
            ".bash_history",
            "home/.Bash_History",
            "gradle.properties",
            "app/Gradle.Properties",
            ".rclone.conf",
            ".yarnrc.yml",
            ".dev.vars",
            "worker/.DEV.VARS",
            "service_account.json",
            "keys/Service_Account.JSON",
            "firebase-adminsdk-x.json",
            "firebase-adminsdk-abc12-9f8e7.json",
            "ssh_host_rsa_key",
            "etc/ssh_host_ed25519_key",
            "private.asc",
            "keys/Private-release.ASC",
            "secret-key.asc",
            "secrets.asc",
            ".m2/settings.xml",
            "home/.m2/settings.xml",
            "build/.M2/Settings.XML",
            ".config/rclone/rclone.conf",
            "x/.config/rclone/rclone.conf",
            ".config/gcloud/credentials.db",
            ".config/gcloud/access_tokens.db",
            ".pip/pip.conf",
            "home/.PIP/Pip.conf",
            ".mc/config.json",
            "home/.mc/config.json",
            ".config/hub",
            "home/.config/hub",
        ] {
            if !secret(path) {
                missed.push(path);
            }
        }
        assert!(missed.is_empty(), "not secret: {missed:?}");
    }

    /// Issue #523: `.` and empty components name nothing, so every spelling of a listed
    /// path is matched as the plain path. A `..` is resolved as well, and a path is a
    /// secret when either its raw or its resolved reading is.
    #[test]
    fn secret_drops_dot_and_empty_components_before_matching() {
        let mut missed = vec![];
        for path in [
            ".docker//config.json",
            ".cargo/./credentials.toml",
            ".config/./gh/hosts.yml",
            "./.my.cnf",
            "a//b/.npmrc",
            "./.docker/config.json",
            ".m2/./settings.xml",
            ".config//rclone/rclone.conf",
            ".config/hub//x",
            ".mc//config.json",
            ".config/gcloud/./credentials.db",
            ".pip/./pip.conf",
            "./src/../.env",
            "a/.docker/x/../config.json",
            ".config/rclone/x/../rclone.conf",
            ".m2/x/../../.m2/settings.xml",
            "a/../.my.cnf",
            "../.docker/config.json",
        ] {
            if !secret(path) {
                missed.push(path);
            }
        }
        assert!(missed.is_empty(), "not secret: {missed:?}");
    }

    /// Issue #523 near misses: ordinary files that share a name stem with the new
    /// credential names, and the same names outside their directories. `docs/*.asc`
    /// stays readable because only `private*.asc` and `secret*.asc` are refused.
    #[test]
    fn secret_leaves_issue_523_near_misses_alone() {
        for path in [
            "README.md",
            "src/auth.rs",
            "auth.rs",
            "auth.json.md",
            "author.json",
            "docs/authors.json",
            "oauth.json",
            "settings.xml",
            "app/settings.xml",
            "m2/settings.xml",
            "docs/.m2/readme.md",
            "config.json",
            "docs/config.json",
            "app/config.json",
            ".mc/config.yml",
            ".mc/config.json.md",
            "docs/mc/config.json",
            "docs/a.asc",
            "docs/signing.asc",
            "keys/public.asc",
            "public.asc",
            "private.asc.md",
            "docs/private-notes.md",
            "docs/secret-plan.md",
            "rclone.conf.md",
            "config/rclone.md",
            "docs/rclone/rclone.conf.md",
            ".config/rclone.md",
            ".config/gcloud/config.json",
            ".config/gcloud/credentials.db.md",
            ".config/hub.md",
            ".config/hubspot.json",
            "hub/x",
            "src/hub",
            ".pip/pip.conf.md",
            "pip.conf.md",
            "terraform.rc.md",
            "src/terraform.rs",
            "terraformrc.md",
            "_netrc.md",
            "gradle.properties.md",
            "src/gradle.rs",
            "firebase.json",
            "firebase-adminsdk.md",
            "firebase-adminsdk-x.json.md",
            "service_account_notes.json",
            "docs/service_account.md",
            "ssh_host_rsa_key.pub",
            "ssh_host_notes.md",
            "kdbx.md",
            "docs/keepass.md",
            "notes.kdbx.md",
            "bash_history.md",
            ".dev.vars.md",
            "./README.md",
            "docs//config.json",
            "./src/auth.rs",
            "src/./author.json",
            "x/../README.md",
            "a/../b/settings.xml",
        ] {
            assert!(!secret(path), "{path} must stay readable");
        }
    }

    /// Issue #540: more shell and database histories, and token files under `.config`.
    #[test]
    fn secret_covers_issue_540_credential_files() {
        let mut missed = vec![];
        for path in [
            ".zsh_history",
            "home/.ZSH_HISTORY",
            ".zhistory",
            ".mysql_history",
            ".psql_history",
            ".python_history",
            ".node_repl_history",
            ".sqlite_history",
            "home/.Sqlite_History",
            ".authinfo",
            "home/.AUTHINFO",
            ".sentryclirc",
            "app/.sentryclirc",
            ".config/git/credentials",
            "home/.Config/Git/Credentials",
            ".config/pip/pip.conf",
            "x/.config/pip/pip.conf",
            ".config/netrc",
            "home/.CONFIG/NETRC",
            ".config/./git//credentials",
            ".config/git/x/../credentials",
        ] {
            if !secret(path) {
                missed.push(path);
            }
        }
        assert!(missed.is_empty(), "not secret: {missed:?}");
    }

    /// Issue #540 near misses. The `.config` chain rules need the leading dot directory,
    /// like `.pip/pip.conf` and `.config/rclone/rclone.conf`: `config/git/credentials`
    /// stays readable.
    #[test]
    fn secret_leaves_issue_540_near_misses_alone() {
        for path in [
            "docs/history.md",
            "history",
            "zsh_history_notes.md",
            "zsh_history",
            "zhistory.md",
            "python_history.md",
            ".zsh_history.md",
            "config/git/credentials-guide.md",
            "config/git/credentials",
            "config/pip/pip.conf",
            "config/netrc",
            ".config/git/config",
            ".config/git/credentials.md",
            ".config/git/credentials-guide.md",
            ".config/pip/pip.conf.md",
            ".config/pip/other.conf",
            ".config/netrc.md",
            ".config/netrcs",
            "docs/.config/git/notes/credentials",
            "authinfo.md",
            "sentryclirc.md",
            ".sentryclirc.md",
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

    /// Issue #505: exactly the three template basenames are exempt, only as the final
    /// component; every other `.env*` name and any directory of that name stays refused.
    #[test]
    fn secret_exempts_only_the_exact_env_template_basenames() {
        for name in [".env.example", ".env.sample", ".env.template"] {
            assert!(!read_secret(name), "{name}");
            assert!(!read_secret(&format!("config/{name}")), "config/{name}");
            assert!(!read_secret(&format!("./{name}")), "./{name}");
            assert!(!read_secret(&format!("a//b/{name}")), "a//b/{name}");
            assert!(!read_secret(&name.to_uppercase()), "{name} upper");
            assert!(!read_excluded(name), "read_excluded {name}");
            // The strict list, used by writes and commits, still refuses every template.
            assert!(secret(name), "strict {name}");
            assert!(excluded(name), "strict excluded {name}");
            assert!(secret(&format!("config/{name}")), "strict config/{name}");
            // A directory with that name does not exempt its contents or `..` escapes.
            assert!(read_secret(&format!("{name}/x.txt")), "{name}/x.txt");
            assert!(read_secret(&format!("{name}/.env")), "{name}/.env");
            assert!(read_secret(&format!("{name}/..")), "{name}/..");
            assert!(read_secret(&format!("{name}/../.env")), "{name}/../.env");
            // Near misses: an added or missing character, or a second suffix.
            assert!(read_secret(&format!("{name}x")), "{name}x");
            assert!(read_secret(&format!("{name}.local")), "{name}.local");
            assert!(read_secret(&format!("{name}.env")), "{name}.env");
            assert!(!read_secret(&format!("x{name}")), "x{name}");
            assert!(
                read_secret(&name[..name.len() - 1]),
                "{}",
                &name[..name.len() - 1]
            );
        }
        for name in [
            ".env",
            ".env.local",
            ".env.production",
            ".env.example.local",
            ".ENV.EXAMPLE.LOCAL",
            "prod.env",
            ".envrc",
            "config/.env.local",
        ] {
            assert!(read_secret(name), "{name}");
        }
    }
}
