### Added

- Coding workers can answer a small change to a large file with a strict
  `=== EDIT: path ===` block of `<<<<<<< SEARCH` / `=======` / `>>>>>>> REPLACE` pairs
  instead of re-emitting the whole file, so files of about 25-64 KB fit the output
  cap. Each SEARCH must match the approved file exactly once; otherwise nothing is
  applied and the failure names the file and pair for the repair. EDIT blocks go
  through the same approved-file, path, secret, NUL and size checks as FILE blocks;
  whole-file answers and the 64 KiB file bound are unchanged.
