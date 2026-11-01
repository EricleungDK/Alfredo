### Added

- `/go` checks can run the cargo toolchain: `cargo test --offline` (or an absolute path
  under `~/.cargo/bin`) passes plan lint and runs in the check sandbox. The sandbox mounts
  the same read-only toolchain entries as chat `run_command` (`~/.cargo/bin`, `registry`,
  `git`, `~/.rustup/toolchains`, `settings.toml`) from the `$HOME` of Alfredo itself, with
  `CARGO_HOME`, `RUSTUP_HOME` and `CARGO_NET_OFFLINE=true`; `credentials.toml`, other
  cargo files and token variables stay hidden and the network stays off. Programs outside
  `/usr/bin:/bin` and the toolchain are still refused with the same message. `--doctor`
  notes the mounted roots (#365).
