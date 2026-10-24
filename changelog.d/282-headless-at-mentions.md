### Fixed

- `--chat PROMPT --headless` now attaches `@path` mentions like interactive chat (same
  rules and bounds), prints one stderr line per mention it could not attach, and adds an
  `"attached"` list to the JSON result when files were attached.
