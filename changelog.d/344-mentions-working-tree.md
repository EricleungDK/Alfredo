### Fixed

- Chat `@file` mentions now attach the file as it is in the working tree, so the text
  after an approved `edit_file` is current and a new untracked file can be attached. The
  heading reads "working tree content". Files use the same checks as `read_file`;
  gitignored untracked files, symlinks (including symlinked directories), secrets,
  binary files and files over 256 KiB are still refused with a notice.
