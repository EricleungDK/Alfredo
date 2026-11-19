use super::Changes;
use crate::tasks::TaskStore;
use std::path::{Path, PathBuf};

/// Compose accepted candidates on the recorded baseline with the same object-only
/// merge used for dependency baselines, then create one new local branch ref.
pub(super) async fn integration_branch(
    store: TaskStore,
    workspace: PathBuf,
    baseline: String,
    sources: Vec<u64>,
    name: String,
    goal: String,
) -> Result<String, String> {
    let mut candidates = Vec::new();
    for source in sources {
        let reader = store.clone();
        let raw = tokio::task::spawn_blocking(move || reader.evidence(source))
            .await
            .map_err(|_| "Evidence reader stopped".to_string())??;
        let evidence: crate::worker::Evidence =
            serde_json::from_str(&raw).map_err(|_| "Malformed accepted evidence")?;
        candidates.push((
            source,
            crate::worker::verify_candidate(&workspace, &evidence).await?,
        ));
    }
    let composed = crate::dependencies::compose(&workspace, baseline, &candidates).await?;
    // The tip names the goal; its tree is the composed tree, so the content is unchanged.
    let subject = crate::worker::commit_line(&goal, 72);
    let subject = if subject.is_empty() {
        "Alfredo autopilot".to_string()
    } else {
        subject
    };
    let message = format!(
        "{subject}\n\nAlfredo autopilot integration of {} accepted task{}",
        candidates.len(),
        if candidates.len() == 1 { "" } else { "s" }
    );
    let commit = crate::worker::git(
        &workspace,
        &[
            "-c",
            "user.name=Alfredo",
            "-c",
            "user.email=alfredo@localhost",
            "-c",
            "commit.gpgSign=false",
            "commit-tree",
            &format!("{composed}^{{tree}}"),
            "-p",
            &composed,
            "-m",
            &message,
        ],
    )
    .await?
    .trim()
    .to_string();
    let reference = format!("refs/heads/{name}");
    match crate::worker::git(&workspace, &["show-ref", "--verify", "--hash", &reference]).await {
        Ok(current) if current.trim() == commit => {}
        // A retry after a restart rebuilds the tip with a new timestamp. Accept a ref
        // sitting on the same composed tip (goal commit on top) or at it (a branch
        // made before the goal commit existed).
        Ok(current)
            if current.trim() == composed
                || crate::worker::git(
                    &workspace,
                    &["rev-parse", &format!("{}^", current.trim())],
                )
                .await
                .is_ok_and(|parent| parent.trim() == composed) =>
        {
            return Ok(current.trim().to_string());
        }
        Ok(_) => {
            return Err(format!(
                "{name} already exists and points elsewhere; it was not changed"
            ))
        }
        Err(_) => {
            crate::worker::git(
                &workspace,
                &[
                    "update-ref",
                    "--no-deref",
                    &reference,
                    &commit,
                    "0000000000000000000000000000000000000000",
                ],
            )
            .await?;
        }
    }
    Ok(commit)
}

/// The totals of `git diff --numstat baseline..commit` as JSON text, through the
/// hardened host Git helper; `None` when Git cannot say, so the row is left out.
pub(super) async fn branch_changes(
    workspace: &Path,
    baseline: &str,
    commit: &str,
) -> Option<String> {
    let range = format!("{baseline}..{commit}");
    let numstat = crate::worker::git(
        workspace,
        &[
            "diff",
            "--numstat",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            &range,
            "--",
        ],
    )
    .await
    .ok()?;
    serde_json::to_string(&Changes::from_numstat(&numstat)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Repo {
        root: PathBuf,
        store: TaskStore,
    }

    impl Repo {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "alfredo-integration-branch-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(root.join("workspace")).unwrap();
            let repo = Self {
                store: TaskStore::new(&root.join("state"), &root.join("workspace"), "mission")
                    .unwrap(),
                root,
            };
            repo.git(&["init", "-q"]);
            repo
        }

        fn workspace(&self) -> PathBuf {
            self.root.join("workspace")
        }

        fn git(&self, args: &[&str]) -> String {
            let output = Command::new("git")
                .current_dir(self.workspace())
                .args([
                    "-c",
                    "user.name=T",
                    "-c",
                    "user.email=t@localhost",
                    "-c",
                    "commit.gpgSign=false",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "git {args:?} failed");
            String::from_utf8(output.stdout).unwrap().trim().to_string()
        }

        fn commit(&self, file: &str) -> String {
            std::fs::write(self.workspace().join(file), file).unwrap();
            self.git(&["add", file]);
            self.git(&["commit", "-q", "-m", file]);
            self.git(&["rev-parse", "HEAD"])
        }

        fn point(&self, branch: &str, commit: &str) {
            self.git(&["update-ref", &format!("refs/heads/{branch}"), commit]);
        }

        fn branch(&self, branch: &str) -> String {
            self.git(&["rev-parse", &format!("refs/heads/{branch}")])
        }

        async fn run(&self, baseline: &str, name: &str) -> Result<String, String> {
            integration_branch(
                self.store.clone(),
                self.workspace(),
                baseline.to_string(),
                Vec::new(),
                name.to_string(),
                "Ship the goal".to_string(),
            )
            .await
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[tokio::test]
    async fn a_new_branch_is_a_goal_commit_on_the_composed_tip() {
        let repo = Repo::new();
        let first = repo.commit("a");
        let baseline = repo.commit("b");
        let commit = repo.run(&baseline, "alfredo/new").await.unwrap();
        assert_ne!(commit, baseline);
        assert_eq!(repo.branch("alfredo/new"), commit);
        assert_eq!(repo.git(&["rev-parse", &format!("{commit}^")]), baseline);
        assert_ne!(repo.git(&["rev-parse", &format!("{commit}^")]), first);
        assert_eq!(
            repo.git(&["log", "-1", "--format=%B", &commit]),
            "Ship the goal\n\nAlfredo autopilot integration of 0 accepted tasks"
        );
    }

    #[tokio::test]
    async fn a_branch_already_at_the_composed_tip_is_accepted_unchanged() {
        let repo = Repo::new();
        repo.commit("a");
        let baseline = repo.commit("b");
        repo.point("alfredo/at-tip", &baseline);
        assert_eq!(
            repo.run(&baseline, "alfredo/at-tip").await,
            Ok(baseline.clone())
        );
        assert_eq!(repo.branch("alfredo/at-tip"), baseline);
    }

    #[tokio::test]
    async fn a_branch_with_a_goal_commit_on_the_composed_tip_is_accepted_unchanged() {
        let repo = Repo::new();
        repo.commit("a");
        let baseline = repo.commit("b");
        let earlier = repo
            .run(&baseline, "alfredo/earlier")
            .await
            .expect("first run creates the branch");
        // Same parent, different message: what a retry after a restart would see.
        let other = repo.git(&[
            "commit-tree",
            &format!("{baseline}^{{tree}}"),
            "-p",
            &baseline,
            "-m",
            "an earlier attempt",
        ]);
        assert_ne!(other, earlier);
        repo.point("alfredo/retry", &other);
        assert_eq!(
            repo.run(&baseline, "alfredo/retry").await,
            Ok(other.clone())
        );
        assert_eq!(repo.branch("alfredo/retry"), other);
    }

    #[tokio::test]
    async fn a_branch_pointing_elsewhere_is_refused_and_left_alone() {
        let repo = Repo::new();
        let first = repo.commit("a");
        let baseline = repo.commit("b");
        // Neither the tip nor a child of it: a child of the tip's parent.
        let sibling = repo.git(&[
            "commit-tree",
            &format!("{baseline}^{{tree}}"),
            "-p",
            &first,
            "-m",
            "elsewhere",
        ]);
        repo.point("alfredo/taken", &sibling);
        let error = repo.run(&baseline, "alfredo/taken").await.unwrap_err();
        assert_eq!(
            error,
            "alfredo/taken already exists and points elsewhere; it was not changed"
        );
        assert_eq!(repo.branch("alfredo/taken"), sibling);
        // A branch on an older commit with no parent either is refused too.
        repo.point("alfredo/old", &first);
        assert!(repo.run(&baseline, "alfredo/old").await.is_err());
        assert_eq!(repo.branch("alfredo/old"), first);
    }
}
