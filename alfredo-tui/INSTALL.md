# Installing alfredo-tui

`alfredo-tui` is a native terminal for Linux x86-64 that runs local Ollama coding
agents on a Git repository. This archive holds a prebuilt binary. It was built on
the GNU/Linux host recorded in `BUILD.json`; older glibc versions are not tested.

## Requirements

- Linux x86-64 (WSL2 works).
- Git at `/usr/bin/git`; a repository with at least one commit.
- Ollama with an installed model (`ollama pull qwen2.5-coder:14b`).
- For coding workers: `/usr/bin/bwrap` (bubblewrap) and `/usr/bin/prlimit`
  (util-linux). Debian/Ubuntu: `sudo apt install git bubblewrap util-linux`.

No Rust, Python, Node or browser is needed to run the binary.

## Install

Checksums detect corruption; they do not authenticate the publisher.

```sh
sha256sum -c alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz
cd alfredo-tui-0.1.0-x86_64-unknown-linux-gnu
mkdir -p "$HOME/.local/bin"
install -m 755 alfredo-tui "$HOME/.local/bin/alfredo-tui"
export PATH="$HOME/.local/bin:$PATH"   # if not already on PATH
alfredo-tui --version
```

## Run

```sh
cd ~/code/my-repo
alfredo-tui --doctor --model qwen2.5-coder:14b   # optional preflight; exit 2 = problems
alfredo-tui --model qwen2.5-coder:14b
```

Type `/go GOAL` to run autopilot, F5 to pause/resume, `/stop` to cancel. When it
finishes, review and merge the local branch it names (`alfredo/go-<id>`) with
normal Git. F1 lists commands; `alfredo-tui --help` lists flags. Ollama is expected
at `http://127.0.0.1:11434`; use `--endpoint URL` otherwise. If `OLLAMA_HOST` is a
bind address such as `0.0.0.0`, startup fails with "Invalid Ollama URL"; pass
`--endpoint`.

## State, upgrade and uninstall

- State lives in `$HOME/.local/state/alfredo` (or `--state-dir` /
  `ALFREDO_STATE_DIR`), outside your repositories. Accepted work also leaves Git
  refs under `refs/alfredo/` and branches named `alfredo/...` in your repository.
- Current formats: task schema 16, conversation schema 17. Older state is upgraded
  on first write, with an exact backup of the original file. Older binaries may
  refuse newer state: back up the state directory before upgrading, and do not
  delete state to force a downgrade.
- Upgrade: keep the previous binary, then install the new one over it.
- Uninstall: remove `~/.local/bin/alfredo-tui`. Remove the state directory only if
  you no longer need task and conversation history.

## Included files

| File | Contents |
| --- | --- |
| `alfredo-tui` | The binary |
| `LICENSE` | MIT license for Alfredo's own code |
| `THIRD_PARTY_NOTICES.txt`, `DEPENDENCIES.json` | Dependency notices and inventory (dependencies keep their own licenses) |
| `Cargo.lock` | Exact dependency resolution |
| `BUILD.json` | Build provenance: compiler, source commit, dirty flag, checksums |

Full documentation, including troubleshooting and detailed behavior, is in the
source repository's `README.md` and `alfredo-tui/docs/`.
