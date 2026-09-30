# Roadmap

**Last Updated**: 2026-09-30

Current state: [STATUS.md](STATUS.md). The product is the native terminal `alfredo-tui`.

## Plans

- [Launch acceptance](launch-acceptance.md)
- [Rust terminal migration](rust-terminal-migration.md) and [regression inventory](rust-terminal-regression-inventory.md)
- [Side pane and agent view](tui-side-pane.md)
- [Command chronology](native-command-chronology.md), [selection chronology](native-selection-chronology.md)
- [Inference admission](native-inference-admission.md), [inference qualification](native-inference-qualification.md)
- [Mission Work tree](native-mission-work-tree.md), [rendering](native-mission-work-rendering.md)
- [Check-result recovery](native-check-result-recovery.md)

## Verification Command

```bash
cargo fmt --manifest-path alfredo-tui/Cargo.toml -- --check
cargo clippy --locked --manifest-path alfredo-tui/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path alfredo-tui/Cargo.toml
(cd alfredo-tui && python3 -m unittest discover -s tests -p 'test_*.py')
```
