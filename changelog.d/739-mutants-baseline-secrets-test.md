### Fixed

- The sandbox secrets test `hidden_note_and_escalate_hint_do_not_contradict` no longer
  fails on runners whose `cat` is uutils coreutils, which exits 2 (GNU `cat` exits 1) when
  two files fail to read. It still requires a failed run and every note and hint check.
