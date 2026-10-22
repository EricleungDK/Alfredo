### Security

- The sandboxed `run_command` no longer exposes the whole `~/.cargo` and `~/.rustup`
  directories. Cargo credentials and config (`~/.cargo/credentials.toml`, `config.toml`)
  can no longer be read from inside the sandbox; only the toolchain entries an offline
  build needs are mounted read-only, so `cargo build` and `cargo test` still work.
- Limit: credentials under `~/.cargo` are hidden from the sandbox unless the Coding
  Workspace itself contains the cargo home (for example a workspace at `$HOME`), because
  the workspace is mounted as is.
