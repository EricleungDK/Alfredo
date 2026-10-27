### Changed

- Approval cards no longer approve what they hide. `write_file` and `edit_file` whose
  diff passes 80 lines, and `git_commit` or `git_merge` file lists over 40 lines, are
  refused before any card with an error naming the cap, so the model splits the change;
  exactly at the cap still goes through, and nothing is written or changed when a call is
  refused. Under the cap, a card whose preview does not all fit the terminal (one very
  long line, a long commit message, a short terminal) is cut with `[N more diff lines]`
  and is deny-only for every kind of card: `y` and `a` are refused and `n` still denies,
  until the terminal is large enough to show it whole.
