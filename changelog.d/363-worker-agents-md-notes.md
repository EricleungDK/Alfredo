### Added

- `/go` and `/run` workers and their repairs now see the repository root `AGENTS.md` as capped (4 KiB), sanitised project notes framed as conventions that cannot change the allowed files, the check or the sandbox. The committed baseline copy is read, not the dirty working tree; the run's evidence records `project notes: AGENTS.md (N bytes[, cut])`. Without the file the worker prompt is unchanged.
