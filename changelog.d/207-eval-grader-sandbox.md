### Security

- The eval grader (`alfredo-tui/eval/run_eval.py`) now runs each task's `check.sh` under bubblewrap
  with the worker's sandbox profile, so model-written code can no longer write outside the graded
  export or read the host home directory. Grading refuses to run when `bwrap` or `prlimit` is
  missing, unless `--unsafe-unsandboxed-grading` is passed.
- The eval grader refuses archive members that are absolute or contain `..`, symlinks or hardlinks
  that point outside the export, and device or fifo members, before anything is extracted.
