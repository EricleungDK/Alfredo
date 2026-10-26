### Changed

- Internal code-health cleanup with no behaviour change: Git spawning in the worker no longer unwraps its pipes, the unused `committed_text` helper is removed, and a few Clippy pedantic warnings are fixed.
