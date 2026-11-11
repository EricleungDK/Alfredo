//! Where the startup selector begins after a failed automatic open (issue #679).
use alfredo_tui::selection::selector_start;
use std::path::PathBuf;

#[test]
fn failed_automatic_open_starts_the_path_box_at_the_given_workspace() {
    let (starting, workspace) = selector_start("/cwd".into(), Some("/repo".into()), true);
    assert_eq!(starting, PathBuf::from("/repo"));
    assert_eq!(workspace, None);
}

#[test]
fn explicit_selection_keeps_the_working_directory_and_the_workspace() {
    let (starting, workspace) = selector_start("/cwd".into(), Some("/repo".into()), false);
    assert_eq!(starting, PathBuf::from("/cwd"));
    assert_eq!(workspace, Some(PathBuf::from("/repo")));
}

#[test]
fn without_a_workspace_the_working_directory_is_kept_either_way() {
    for automatic in [true, false] {
        let (starting, workspace) = selector_start("/cwd".into(), None, automatic);
        assert_eq!(starting, PathBuf::from("/cwd"));
        assert_eq!(workspace, None);
    }
}
