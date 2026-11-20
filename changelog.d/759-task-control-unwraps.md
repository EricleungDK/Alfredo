### Changed

- Internal: `task_control.rs` no longer uses `.unwrap()` for its three guarded lookups. The task-state and Architect-review checks now bind the value they test, with the same refusal texts as before.
