### Changed

- Approval cards no longer cut what you are asked to approve. `write_file` and `edit_file`
  whose diff passes the 80-line card limit, and `git_commit` or `git_merge` file lists
  over 40 lines, are refused before any card with an error naming the cap, so the model
  splits the change. Exactly at the cap still goes through; nothing is written or
  changed when a call is refused.
