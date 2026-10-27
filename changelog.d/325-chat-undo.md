### Added

- `/undo` restores the files the last chat turn's approved `write_file` and `edit_file`
  calls changed (and deletes the files it created) from pre-image copies kept in the
  State Directory. It refuses, and names, any file changed since, a symlink, a read-only
  file or an excluded path, and says when a file was too large to keep a copy of. It is
  owner-typed only; `run_command` and Git are not covered.
