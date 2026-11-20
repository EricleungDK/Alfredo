### Changed

- The known-flag list, the `--chat -` stdin prompt reader and the selection-failure message moved from `src/main.rs` to `src/cli_support.rs`, so the entry point stays under its line budget. Behaviour is unchanged.
