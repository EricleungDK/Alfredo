### Changed

- A chat `write_file` or `edit_file` whose diff is over one screen (and over 80 lines) is
  no longer refused: the approval card pages it with PageDown / PageUp, `y` and `a` stay
  off until the last page has been shown, the 0.5 s arm delay restarts there, and approval
  writes exactly the bytes that were shown (still refused if the file changed since the
  preview). Creating a 400-line file through chat now works. A card that cannot be paged
  (one line taller than a page, or a terminal too short for a page) stays deny-only, and
  resizing starts the paging and the arm delay over, and a page drawn under the narrow-terminal
  side-pane overlay does not count as shown. A diff over 2000 lines is still refused, `git_*` and
  push cards keep their caps, and an always-allow rule or the headless `--approve` policy
  still never applies a diff over 80 lines unseen.
