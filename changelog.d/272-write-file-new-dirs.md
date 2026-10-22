### Added

- `write_file` can create a file under directories that do not exist yet. The approval
  card lists them (`new directories: src/util`); they are made one level at a time only
  after approval, never through a symlink, into `.git`, secret or generated paths, or
  outside the workspace, and the call is refused if an ancestor changed since the card.
