### Fixed

- The approval card now counts the diff preview in display columns, not characters, so a
  `write_file` or `edit_file` preview of Chinese, Japanese, Korean or emoji text no longer
  grows past the screen. The header, args and key line are always drawn, so `y`, `a` and
  `n` arm instead of answering "Read the approval card first" (issue #410).
