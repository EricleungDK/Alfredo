use alfredo_tui::autopilot::integration_branch;
use alfredo_tui::tasks::TaskStore;
use std::path::PathBuf;
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
            store: TaskStore::new(&root.join("state"), &root.join("workspace"), "mission").unwrap(),
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
