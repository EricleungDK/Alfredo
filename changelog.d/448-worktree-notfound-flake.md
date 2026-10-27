### Fixed

- The `chat_git_tools` worktree tests no longer fail now and then with `NotFound`: they
  install hand-built worktree entries in one rename and turn off git's automatic
  maintenance, which could prune an entry that was still half built (issue #448).
