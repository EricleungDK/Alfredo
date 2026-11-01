//! Plan findings a human would catch reading each policy: checks that can only
//! fail because they need files nothing provides, missing check programs, and
//! unordered tasks writing the same file. Conservative: an argument that cannot
//! be confidently classified as a repository path is never reported.
use crate::{chat_tools::git_read, planner::Plan, worker::Toolchain};
use std::{cell::OnceCell, collections::BTreeSet, path::Path};

/// Most bytes of git output one lint query reads: a listing of one path or a size.
const QUERY_LIMIT: usize = 64 << 10;

/// Read-only roots bound into the check sandbox (`worker::check_request`).
const SANDBOX_ROOTS: [&str; 6] = ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc"];
/// PATH the check sandbox runs with.
const SANDBOX_PATH: [&str; 2] = ["/usr/bin", "/bin"];
const EXTENSIONS: [&str; 38] = [
    "py", "pyi", "js", "mjs", "cjs", "ts", "tsx", "jsx", "sh", "bash", "rb", "go", "rs", "java",
    "kt", "c", "h", "cc", "cpp", "hpp", "cs", "php", "pl", "lua", "json", "toml", "yaml", "yml",
    "ini", "cfg", "txt", "md", "csv", "html", "css", "xml", "sql", "r",
];

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// Whether the check sandbox can resolve `program`. Bare names resolve only on the
/// sandbox PATH: the system directories, then the owner's mounted cargo `bin`. Absolute
/// paths only under the bound system roots or the mounted toolchain entries. A path
/// relative to the worktree cannot be resolved here and is assumed available.
pub fn sandbox_program(program: &str) -> bool {
    sandbox_program_with(program, &Toolchain::owned())
}

/// [`sandbox_program`] against an explicit toolchain: the same mounts the check
/// sandbox gets from [`Toolchain::mounts`], so the lint cannot accept a program the
/// sandbox cannot run (issue #365).
fn sandbox_program_with(program: &str, toolchain: &Toolchain) -> bool {
    if program.starts_with('/') {
        let path = Path::new(program);
        let system = SANDBOX_ROOTS
            .iter()
            .any(|root| program.starts_with(&format!("{root}/")));
        return (system && executable(path)) || mounted_executable(path, toolchain);
    }
    if program.contains('/') {
        return true;
    }
    SANDBOX_PATH
        .iter()
        .any(|dir| executable(&Path::new(dir).join(program)))
        || toolchain
            .bin_dir()
            .is_some_and(|dir| mounted_executable(&Path::new(&dir).join(program), toolchain))
}

/// An executable inside a mounted toolchain entry, by its name and by where it
/// resolves: a `..` or a symlink cannot lead out of the mounts.
fn mounted_executable(path: &Path, toolchain: &Toolchain) -> bool {
    let inside = |candidate: &Path| {
        toolchain
            .binds
            .iter()
            .any(|bind| candidate.starts_with(bind))
    };
    inside(path) && path.canonicalize().is_ok_and(|real| inside(&real)) && executable(path)
}

fn plain(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.chars().any(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(
                    c,
                    ':' | '*'
                        | '?'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '$'
                        | '\''
                        | '"'
                        | '`'
                        | '\\'
                        | '='
                        | ';'
                        | '('
                        | ')'
                        | '<'
                        | '>'
                        | '|'
                        | '&'
                        | '~'
                        | ','
                )
        })
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// A relative file or directory path, normalized, or None when unsure.
fn file_path(arg: &str) -> Option<String> {
    if arg.starts_with('-') {
        return None;
    }
    let arg = arg.strip_prefix("./").unwrap_or(arg);
    let slash = arg.contains('/');
    let arg = arg.trim_end_matches('/');
    if !plain(arg) {
        return None;
    }
    let name = arg.rsplit('/').next().unwrap_or(arg);
    let known = name
        .rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && EXTENSIONS.contains(&ext));
    (slash || known).then(|| arg.to_string())
}

/// Candidate paths for a unittest module name (`a.b.C` may be `a/b.py` + class `C`).
fn module_paths(arg: &str) -> Option<Vec<String>> {
    let parts: Vec<_> = arg.split('.').collect();
    if !parts.iter().all(|part| {
        part.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
            && part.chars().all(|c| c.is_alphanumeric() || c == '_')
    }) {
        return None;
    }
    let mut candidates = vec![];
    for end in (1..=parts.len()).rev() {
        let base = parts[..end].join("/");
        candidates.push(format!("{base}.py"));
        candidates.push(base);
    }
    Some(candidates)
}

/// Each required reference as alternatives; the first names it in findings.
fn references(check: &[String]) -> Vec<Vec<String>> {
    let python = check
        .first()
        .and_then(|p| p.rsplit('/').next())
        .is_some_and(|name| name.starts_with("python"));
    let mut unittest = false;
    let discover = check.iter().any(|arg| arg == "discover");
    let mut result = vec![];
    let mut skip = false;
    for (index, arg) in check.iter().enumerate().skip(1) {
        if skip {
            skip = false;
            continue;
        }
        if python && arg == "-m" {
            unittest = check.get(index + 1).is_some_and(|m| m == "unittest");
            skip = true;
            continue;
        }
        if matches!(arg.as_str(), "-c" | "-e" | "-o" | "--output" | "-k") {
            skip = true;
            continue;
        }
        if let Some(path) = file_path(arg) {
            result.push(vec![path]);
        } else if unittest && !discover && !arg.starts_with('-') {
            if let Some(paths) = module_paths(arg) {
                result.push(paths);
            }
        }
    }
    result
}

const DATA_EXTENSIONS: [&str; 8] = [
    "json", "jsonl", "db", "sqlite", "sqlite3", "csv", "pkl", "log",
];
const FIXTURE_MARKERS: [&str; 8] = [
    "fixture",
    "testdata",
    "test_data",
    "sample",
    "example",
    "expected",
    "mock",
    "snapshot",
];

/// A data file a program writes at runtime (`todo.json`, `app.sqlite`), as
/// opposed to a manifest, config or test fixture that is source-like work.
fn runtime_data(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    let Some((_, extension)) = name.rsplit_once('.') else {
        return false;
    };
    if !DATA_EXTENSIONS.contains(&extension) {
        return false;
    }
    let manifest = matches!(
        name,
        "package.json" | "composer.json" | "deno.json" | "manifest.json" | "renovate.json"
    );
    let config = ["config", "rc.", "lock.", "schema"]
        .iter()
        .any(|marker| name.contains(marker));
    let fixture = FIXTURE_MARKERS.iter().any(|marker| lower.contains(marker));
    !(manifest || config || fixture)
}

/// All findings for `plan`, using `tracked` (a committed file or directory at
/// the plan baseline) and `program` (resolvable in the check sandbox).
pub fn lint(
    plan: &Plan,
    tracked: &dyn Fn(&str) -> bool,
    program: &dyn Fn(&str) -> bool,
    size: &dyn Fn(&str) -> Option<u64>,
) -> Vec<String> {
    let mut findings = vec![];
    // closure[i]: task indexes task i transitively depends on.
    let mut closure: Vec<BTreeSet<usize>> = vec![];
    for step in &plan.tasks {
        let mut set = BTreeSet::new();
        for dependency in &step.dependencies {
            let index = (*dependency as usize).wrapping_sub(1);
            if let Some(inner) = closure.get(index) {
                set.insert(index);
                set.extend(inner.iter().copied());
            }
        }
        closure.push(set);
    }
    for (index, step) in plan.tasks.iter().enumerate() {
        let number = index + 1;
        if step.policy.files.is_empty() {
            findings.push(format!(
                "Task {number} lists no policy files; every task must write at least one file."
            ));
        }
        for file in &step.policy.files {
            if let Some(bytes) =
                size(file).filter(|bytes| *bytes > crate::worker::MAX_APPROVED_FILE)
            {
                findings.push(crate::worker::file_too_large(file, bytes));
            }
            if runtime_data(file) && !tracked(file) {
                findings.push(format!(
                    "Task {number} lists runtime data file {file}. Files a program writes when it runs become tracked work, and a later check that rewrites them is refused as modifying files outside approved paths. Do not list it: have the code and tests create it in a temp dir (or have the check use a temp path) and list only source, test and config files."
                ));
            }
        }
        if let Some(name) = step.policy.check.first() {
            if name.chars().any(char::is_whitespace) {
                findings.push(format!(
                    "Task {number} check must be an argv array, not a shell string: split {:?} into separate program and argument strings",
                    crate::autopilot::clean(name, 200)
                ));
            } else if !program(name) {
                findings.push(format!(
                    "Task {number} check program {:?} is not available in the check sandbox (PATH /usr/bin:/bin); use a program installed there, such as python3",
                    crate::autopilot::clean(name, 200)
                ));
            }
        }
        let written: Vec<&String> = std::iter::once(index)
            .chain(closure[index].iter().copied())
            .flat_map(|task| plan.tasks[task].policy.files.iter())
            .collect();
        let provided = |path: &str| {
            tracked(path)
                || written
                    .iter()
                    .any(|file| *file == path || file.starts_with(&format!("{path}/")))
        };
        for alternatives in references(&step.policy.check) {
            if !alternatives.iter().any(|path| provided(path)) {
                findings.push(format!(
                    "Task {number} check references {}, which does not exist yet and is not written by task {number} or its dependencies. Either write it in the same task, make the task depend on the task that writes it, or use a check that only needs this task's files.",
                    alternatives[0]
                ));
            }
        }
    }
    for (later, step) in plan.tasks.iter().enumerate() {
        for earlier in 0..later {
            if closure[later].contains(&earlier) {
                continue;
            }
            if let Some(file) = step
                .policy
                .files
                .iter()
                .find(|file| plan.tasks[earlier].policy.files.contains(file))
            {
                findings.push(format!(
                    "Tasks {} and {} both write {file} but neither depends on the other. Merge them or make one depend on the other.",
                    earlier + 1,
                    later + 1
                ));
            }
        }
    }
    findings
}

/// Whether git may be asked about the plan's workspace. The read tier's refusal (an include
/// that git would open, a filter driver, a config it cannot read) makes every git answer
/// unknown: a FIFO include would stall git (#519). Checked once per lint, before its first
/// git query.
fn repository_trusted(plan: &Plan) -> bool {
    plan.scope
        .as_ref()
        .is_some_and(|scope| git_read::refuse_filters(&scope.workspace).is_ok())
}

/// Committed paths at the plan's pinned baseline, from its context and, when
/// that listing was truncated, from Git. Unknown answers count as present.
fn committed<'a>(plan: &'a Plan, trusted: &'a dyn Fn() -> bool) -> impl Fn(&str) -> bool + 'a {
    move |path: &str| {
        let Some(context) = &plan.context else {
            return true;
        };
        let prefix = format!("{path}/");
        if context
            .paths
            .iter()
            .any(|known| known == path || known.starts_with(&prefix))
        {
            return true;
        }
        if context.omitted_paths == 0 {
            return false;
        }
        let Some(scope) = &plan.scope else {
            return true;
        };
        if !trusted() {
            return true;
        }
        git_read::git_limited(
            &scope.workspace,
            &[
                "--literal-pathspecs",
                "ls-tree",
                "--name-only",
                "-z",
                context.baseline.as_str(),
                "--",
                path,
            ],
            QUERY_LIMIT,
        )
        .map_or(true, |listing| !listing.is_empty())
    }
}

/// Findings against the real baseline and check sandbox. Host git runs within the read
/// tier's budget (one shared deadline for the whole lint), after its include and filter
/// refusal; a refused or timed-out answer counts as unknown (#519).
pub fn findings(plan: &Plan) -> Vec<String> {
    git_read::call_scope(git_read::DEADLINE, || {
        let checked = OnceCell::new();
        let trusted = || *checked.get_or_init(|| repository_trusted(plan));
        let findings = lint(
            plan,
            &committed(plan, &trusted),
            &sandbox_program,
            &committed_size(plan, &trusted),
        );
        findings
    })
}

/// Byte size of a committed file at the plan's pinned baseline; `None` when the
/// plan has no baseline or the path is not a blob there.
fn committed_size<'a>(
    plan: &'a Plan,
    trusted: &'a dyn Fn() -> bool,
) -> impl Fn(&str) -> Option<u64> + 'a {
    move |path: &str| {
        let context = plan.context.as_ref()?;
        let scope = plan.scope.as_ref()?;
        if !trusted() {
            return None;
        }
        let object = format!("{}:{path}", context.baseline);
        git_read::git_limited(
            &scope.workspace,
            &["--literal-pathspecs", "cat-file", "-s", &object],
            QUERY_LIMIT,
        )
        .ok()?
        .trim()
        .parse()
        .ok()
    }
}

#[cfg(test)]
mod toolchain_tests {
    use super::{sandbox_program_with, Toolchain};
    use crate::worker::toolchain_binds;
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};

    fn script(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn home(tag: &str) -> std::path::PathBuf {
        let home = std::env::temp_dir().join(format!(
            "alfredo-lint-toolchain-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&home);
        script(&home.join(".cargo/bin/cargo"));
        script(&home.join(".cargo/hook"));
        script(&home.join(".rustup/toolchains/stable/bin/rustc"));
        fs::create_dir_all(home.join(".cargo/registry")).unwrap();
        home.canonicalize().unwrap()
    }

    #[test]
    fn programs_resolve_exactly_where_the_toolchain_is_mounted() {
        let home = home("mounted");
        let mounted = toolchain_binds(Some(&home));
        let none = Toolchain::default();
        let cargo = home.join(".cargo/bin/cargo");
        let cargo = cargo.to_str().unwrap();
        let rustc = home.join(".rustup/toolchains/stable/bin/rustc");
        let rustc = rustc.to_str().unwrap();
        // Accepted iff mounted: the bare name and the absolute path.
        assert!(sandbox_program_with("cargo", &mounted));
        assert!(!sandbox_program_with("cargo", &none));
        assert!(sandbox_program_with(cargo, &mounted));
        assert!(!sandbox_program_with(cargo, &none));
        assert!(sandbox_program_with(rustc, &mounted));
        assert!(!sandbox_program_with(rustc, &none));
        // Bare names resolve on the cargo bin only, not in the rustup toolchain's.
        assert!(!sandbox_program_with("rustc", &mounted));
        // The system PATH is unaffected by the toolchain.
        for toolchain in [&mounted, &none] {
            assert!(sandbox_program_with("sh", toolchain));
            assert!(sandbox_program_with("/bin/sh", toolchain));
            assert!(!sandbox_program_with(
                "alfredo-no-such-program-x9",
                toolchain
            ));
            assert!(!sandbox_program_with("/opt/alfredo/bin/tool", toolchain));
            assert!(sandbox_program_with("scripts/check.sh", toolchain));
        }
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn a_path_cannot_leave_the_mounted_entries() {
        let home = home("escape");
        let mounted = toolchain_binds(Some(&home));
        let hook = home.join(".cargo/hook");
        assert!(!sandbox_program_with(hook.to_str().unwrap(), &mounted));
        let climbing = format!("{}/.cargo/bin/../hook", home.display());
        assert!(!sandbox_program_with(&climbing, &mounted));
        // A symlink inside a mounted entry that points outside the mounts.
        std::os::unix::fs::symlink(&hook, home.join(".cargo/bin/linked")).unwrap();
        let linked = home.join(".cargo/bin/linked");
        assert!(!sandbox_program_with(linked.to_str().unwrap(), &mounted));
        assert!(!sandbox_program_with("linked", &mounted));
        // The registry is mounted, so an executable inside it resolves.
        let tool = home.join(".cargo/registry/tool");
        script(&tool);
        assert!(sandbox_program_with(tool.to_str().unwrap(), &mounted));
        // Not executable, or absent: refused.
        fs::write(home.join(".cargo/bin/data"), "x").unwrap();
        assert!(!sandbox_program_with("data", &mounted));
        assert!(!sandbox_program_with("missing", &mounted));
        let _ = fs::remove_dir_all(home);
    }
}
