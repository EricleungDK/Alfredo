### Fixed

- Chat `grep` and `list_files` take an optional `glob` (`*`, `?`, `**`) that filters the
  returned paths, a zero-match `grep` for a pattern with regex syntax now says it matches
  literal text, a `path` containing a glob character points to `glob`, and an unknown
  argument name produces a note instead of being silently ignored (issue #337).
