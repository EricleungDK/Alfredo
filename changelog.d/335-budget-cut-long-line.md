### Fixed

- A budgeted tool-result cut no longer backs up to a newline that would discard half the
  budget, so a short header followed by one huge line (`run_command`, `git_diff` of a
  minified file) keeps its body under a small `--num-ctx`.
