//! Issue #631: the F1 help names Ctrl+O for the worker's code in task detail.

#[test]
fn f1_help_title_names_ctrl_o_for_worker_code() {
    let title = alfredo_tui::ui::HELP_BOTTOM_TITLE;
    assert!(title.contains("^O code"), "{title}");
    assert!(title.contains("F3 evidence"), "{title}");
}
