# Contributing to Alfredo

The product is the native terminal in [`alfredo-tui/`](alfredo-tui/). The Python
`albert_mvp/` and React/Tauri `mission-control/` code is [legacy](docs/legacy.md);
`mission-control/src-tauri/src/execution.rs` is still compiled into the terminal, so
keep it building.

## Set up

Linux x86-64 (WSL2 works, inside the Linux filesystem), Git, Rust 1.96.0, and
`sudo apt install bubblewrap util-linux` for the worker sandbox. Python 3 is a
test-only dependency.

```bash
git clone https://github.com/EricleungDK/Alfredo.git && cd Alfredo
cargo build --locked --manifest-path alfredo-tui/Cargo.toml
```

## Workflow

1. Open or pick a [GitHub issue](https://github.com/EricleungDK/Alfredo/issues) first
   for anything beyond a small fix.
2. Test first: write the failing test, make it pass, then refactor. Rust behavior
   goes in `alfredo-tui/tests/*.rs`; terminal journeys in `alfredo-tui/tests/*_smoke.py`.
3. Run the same gates as CI before you push:

```bash
cargo fmt --manifest-path alfredo-tui/Cargo.toml -- --check
cargo clippy --locked --manifest-path alfredo-tui/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path alfredo-tui/Cargo.toml
cargo build --locked --manifest-path alfredo-tui/Cargo.toml
python3 -m unittest discover -s alfredo-tui/tests -p 'test_*.py'
python3 alfredo-tui/tests/terminal_smoke.py    # plus the other *_smoke.py; see alfredo-tui/README.md#test
```

4. Update the docs in the same change (below), then open a pull request against `main`.

## Docs

| Change | Update |
| --- | --- |
| Flag, key or slash command | `README.md` tables, `alfredo-tui/README.md`, `--help` text in `src/main.rs` |
| User-visible behavior | `CHANGELOG.md` under `[Unreleased]`, `alfredo-tui/docs/reference.md` |
| Project state, release readiness | `.agent/Tasks/STATUS.md` |
| Domain vocabulary | `CONTEXT.md` |

`alfredo-tui/tests/test_docs.py` fails when a documented link is broken or the README
misses a CLI flag. Commit messages: short imperative subject, conventional prefix
(`feat`, `fix`, `docs`, `test`).

## Ground rules

- Local first: nothing leaves the machine and Alfredo never pushes or moves the
  user's branch. Changes that weaken this need an issue and an ADR under `docs/adr/`.
- Workers stay sandboxed; do not widen a task's files or check on the user's behalf.

By contributing you agree your work is released under the [MIT License](LICENSE).
