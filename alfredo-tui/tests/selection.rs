use alfredo_tui::selection::acknowledge;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};
static ID: AtomicU64 = AtomicU64::new(0);
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-selection-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn exact_repository_selection_and_creation_do_not_open_mission_state() {
    let f = Fixture::new();
    let state = f.0.join("runtime");
    let target = f.0.join("repository with spaces 界");
    let accepted = acknowledge(&target, &state, true).await.unwrap();
    assert_eq!(accepted, target);
    assert!(target.join(".git").is_dir());
    assert!(!state.exists());
    let context = alfredo_tui::planning_context::capture(&accepted, "Build a new app")
        .await
        .unwrap();
    assert!(context.paths.is_empty());
    assert!(context.sources.is_empty());
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    let parents = std::process::Command::new("git")
        .arg("-C")
        .arg(&target)
        .args(["rev-list", "--parents", "-n", "1", "HEAD"])
        .output()
        .unwrap();
    assert!(parents.status.success());
    assert_eq!(
        String::from_utf8(parents.stdout)
            .unwrap()
            .split_whitespace()
            .count(),
        1
    );
    assert_eq!(acknowledge(&target, &state, false).await.unwrap(), accepted);
    assert!(!state.exists());
    assert!(acknowledge(&target, &state, true)
        .await
        .unwrap_err()
        .contains("already exists"));
    let child = target.join("src");
    fs::create_dir(&child).unwrap();
    assert!(acknowledge(&child, &state, false)
        .await
        .unwrap_err()
        .contains("exact repository root"));
    let nested = target.join("nested");
    assert!(acknowledge(&nested, &state, true).await.is_err());
    assert!(!nested.exists());
}

#[tokio::test]
async fn rejected_targets_preserve_files_and_runtime_separation() {
    let f = Fixture::new();
    let state = f.0.join("runtime");
    let target = f.0.join("existing.txt");
    fs::write(&target, "untouched").unwrap();
    for create in [false, true] {
        assert!(acknowledge(&target, &state, create).await.is_err());
    }
    assert_eq!(fs::read_to_string(&target).unwrap(), "untouched");
    let missing = f.0.join("new");
    assert!(acknowledge(&missing, &missing.join("runtime"), true)
        .await
        .is_err());
    assert!(!missing.exists());
    assert!(acknowledge(&f.0, &state, false).await.is_err());
    assert!(!state.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn aliased_runtime_inside_new_repository_is_rejected_before_creation() {
    let f = Fixture::new();
    let alias = f.0.join("alias");
    std::os::unix::fs::symlink(&f.0, &alias).unwrap();
    let target = f.0.join("new");
    let error = acknowledge(&target, &alias.join("new/runtime"), true)
        .await
        .unwrap_err();
    assert!(error.contains("outside"), "{error}");
    assert!(
        !target.exists(),
        "Rejected selection created repository files"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn unresolved_runtime_ancestors_do_not_create_repository_files() {
    let f = Fixture::new();
    let target = f.0.join("new");
    let dangling = f.0.join("dangling");
    std::os::unix::fs::symlink(f.0.join("absent"), &dangling).unwrap();
    assert!(acknowledge(&target, &dangling.join("runtime"), true)
        .await
        .is_err());
    assert!(!target.exists());
    let file = f.0.join("file");
    fs::write(&file, "untouched").unwrap();
    assert!(acknowledge(&target, &file.join("runtime"), true)
        .await
        .is_err());
    assert!(!target.exists());
    assert_eq!(fs::read_to_string(&file).unwrap(), "untouched");
}

#[tokio::test]
async fn opening_an_existing_unborn_repository_never_creates_a_commit_or_stages_files() {
    let f = Fixture::new();
    let target = f.0.join("existing");
    fs::create_dir(&target).unwrap();
    assert!(std::process::Command::new("git")
        .arg("-C")
        .arg(&target)
        .args(["init", "-q", "--template="])
        .status()
        .unwrap()
        .success());
    fs::write(target.join("draft.txt"), "Keep my draft").unwrap();
    acknowledge(&target, &f.0.join("state"), false)
        .await
        .unwrap();
    let head = std::process::Command::new("git")
        .arg("-C")
        .arg(&target)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .unwrap();
    assert!(!head.status.success());
    assert_eq!(
        fs::read_to_string(target.join("draft.txt")).unwrap(),
        "Keep my draft"
    );
    assert!(!target.join(".git/index").exists());
}

#[tokio::test]
async fn preparing_new_repository_is_read_only_and_captures_resolved_identity() {
    use alfredo_tui::{selection::prepare_workspace, selection_command::WorkspaceChoice};
    let fixture = Fixture::new();
    let target = fixture.0.join("future repository");
    let state = fixture.0.join("future state");
    let choice = prepare_workspace(&target, &state, true).await.unwrap();
    assert_eq!(
        choice,
        WorkspaceChoice::Create {
            parent: fixture.0.canonicalize().unwrap(),
            name: "future repository".into(),
        }
    );
    assert!(!target.exists());
    assert!(!state.exists());
    // Discarding this choice is the picker's cancellation path: it owns no effects.
    drop(choice);
    assert!(!target.exists());
    assert!(!state.exists());
}

#[tokio::test]
async fn mission_preflight_can_correct_resume_or_new_without_creating_state() {
    use alfredo_tui::tasks::TaskStore;
    let fixture = Fixture::new();
    let target = fixture.0.join("workspace");
    let state = fixture.0.join("state");
    acknowledge(&target, &state, true).await.unwrap();
    let store = TaskStore::new(&state, &target, "mission").unwrap();
    assert!(store
        .check_mission_selection(false)
        .unwrap_err()
        .contains("does not exist"));
    store.check_mission_selection(true).unwrap();
    assert!(!state.exists());
    store.select_mission(true).unwrap();
    store.check_mission_selection(false).unwrap();
    assert!(store
        .check_mission_selection(true)
        .unwrap_err()
        .contains("already exists"));
}
