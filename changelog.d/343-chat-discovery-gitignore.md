### Fixed

- Chat `list_files` and `grep` now skip files git ignores (`.gitignore` at any depth,
  `.git/info/exclude`, the global excludes file), so a Python repo's `.venv` no longer
  fills the 500-file and 200-match caps before any project file. Tracked files stay
  visible, `.venv`, `venv`, `__pycache__` and `site-packages` are skipped even without a
  `.gitignore`, and naming an ignored directory in `path` still lists it with a note.
  Outside git, or when git cannot list, the plain walk is the fallback.
