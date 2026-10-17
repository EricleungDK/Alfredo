### Fixed

- A task command that is refused before anything is written (unknown task id, a task
  that has not run, `/branch` before `/accept`) is now shown as `✗ Not dispatched · …`
  instead of `? Outcome unconfirmed`, and no longer suggests `/retry-task` or `/refresh`,
  which could only repeat the same refusal. The message names the next step, for example
  `Task #1 has not run; /run 1 first` or `Unknown task 9; the task panel lists ids`.
  `/retry-task` is suggested only when the outcome is genuinely unknown, such as a failed
  save or a stopped storage worker.
