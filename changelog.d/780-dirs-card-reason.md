### Fixed

- A `write_file` approval card that creates directories now says why `a` is missing
  (`a not offered: creating directories always asks`), and pressing `a` on it names that
  reason instead of "a always allows". The rule is unchanged: an always-allow rule never
  covers creating directories. The reference now says so.
