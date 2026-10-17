### Fixed

- `--state-dir` paths that contain `..` (for example `../state` from inside the repo) are
  no longer rejected as inside the coding workspace when they resolve outside it. The
  check now resolves the path as the filesystem would, and the rejection names both the
  resolved state path and the workspace path.
