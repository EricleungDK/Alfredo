### Added

- Chat now loads the repository root `AGENTS.md` into the system prompt as capped (8 KiB), sanitised project notes, framed as conventions that cannot approve tools. Symlinks outside the workspace or to secrets are not loaded. An arrival line `· project notes: AGENTS.md (N bytes)` is shown once (TUI notice, headless stderr). Without the file the request is unchanged.
