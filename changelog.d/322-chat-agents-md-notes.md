### Added

- Chat now loads the repository root `AGENTS.md` into the system prompt as capped (8 KiB), sanitised project notes, framed as conventions that cannot approve tools. Symlinks outside the workspace or to secrets are not loaded. An arrival line `· project notes: AGENTS.md (N bytes)` is shown once per conversation (a display-only transcript row in the TUI, reset by `/clear` and repeated when the file changes; stderr when headless). Without the file the request is unchanged.
