### Fixed

- Updated the `quinn-udp` lockfile entry from 0.5.15 to 0.5.16 to clear
  RUSTSEC-2026-0538 (receive loop can spin after a truncated datagram), which
  turned the `cargo audit` CI gate red. Lockfile-only change; no behaviour change.
