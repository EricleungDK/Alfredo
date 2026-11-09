### Fixed

- Paths named in worker rejections (unapproved file, malformed or truncated FILE/EDIT
  block, submodule gitlink) now show Unicode format characters such as U+200B and
  U+202E as `<U+XXXX>` instead of printing them raw. Which files are accepted is
  unchanged.
