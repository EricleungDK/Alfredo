### Changed
- Internal refactor, no behaviour change: `ui.rs` is split into `ui/inspector.rs` and `ui/activity_lines.rs`, and the command preparation, dispatch and parsing code of `task_control.rs` moves to `task_control/commands.rs`; the largest module (M15) drops from 3159 to 2517 non-test lines.
