### Changed

- The `write_file` / `edit_file` approval card now shows minimal hunks: one
  `@@ line N @@` header per run of changed lines and only the changed lines, instead of
  every line between the first and the last change. A rewrite that changes lines 1 and
  100 of a 100-line file now fits the 80-line card (it was refused as a 201-line diff).
  Every changed line is still shown, the 80-line cap still refuses larger diffs, and the
  approved bytes are still exactly the previewed ones.
