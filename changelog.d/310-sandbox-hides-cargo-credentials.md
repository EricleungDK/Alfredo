### Security

- The sandboxed `run_command` no longer exposes the whole `~/.cargo` and `~/.rustup`
  directories. Cargo credentials and config (`~/.cargo/credentials.toml`, `config.toml`)
  can no longer be read from inside the sandbox; only the toolchain entries an offline
  build needs are mounted read-only, so `cargo build` and `cargo test` still work.
