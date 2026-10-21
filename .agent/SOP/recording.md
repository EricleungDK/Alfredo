# README recording (GIF, MP4, WebM, poster)

Reproduce: `python3 alfredo-tui/recording/record.py` (writes `docs/assets/tui-recording.*` and
`tui-recording.json`).

- Needs `vhs` 0.12.1 and `ttyd` 1.7.7 on `PATH` (release binaries into `~/.local/bin`), ffmpeg,
  Pillow, Ollama with `qwen2.5-coder:14b`, and a release build.
- Real run: `demo.tape` drives VHS at 2560x1440 against a sample repo made by `sample_repo.sh`
  (HOME `/tmp/demo-home`, prompt `$ `, author "Demo"; no personal paths in frames).
- Model waits are VHS `Hide`..`Show` gaps; `record.py` labels them ("model planning - N s cut").
  The goal is typed unrecorded (appears at once, like a paste).
- VHS captures slower than its 25 fps at this size, so `record.py` retimes frames to wall clock
  (`timeline.retime`); a take is kept only if autopilot repaired a failure and accepted everything,
  otherwise it retries (`--takes`, default 4; the 14b model is not deterministic).
- Budgets enforced by `timeline.problems`: 15-30 s, 15-25 fps, GIF < 5 MB, width >= 1280.
- Pure helpers are unit-tested: `python3 -m unittest alfredo-tui/tests/test_recording.py`.
- After a UI change, rerun, inspect frames for legibility and leaks, commit the assets.
