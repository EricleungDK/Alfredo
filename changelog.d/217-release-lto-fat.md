### Changed

- The release profile now uses `lto = "fat"` and `codegen-units = 1`, shrinking the
  release binary from about 10.8 MB to about 8.7 MB (RSS also drops slightly). Release
  builds may take somewhat longer (about 4 minutes on 2 CPUs).
