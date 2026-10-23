### Security

- Bump `tokio` from 1.53.1 to 1.54.1 in `Cargo.lock` to resolve RUSTSEC-2026-0386
  (unsound `AsyncFd::new`); `cargo audit --deny warnings` passes again.
