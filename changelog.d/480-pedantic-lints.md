### Changed

- Internal code-health: cleared newly introduced clippy pedantic lints (missing semicolons, doc backticks, redundant closures, `clone_from`, `write!` instead of `push_str(&format!(...))`); no user-visible change.
