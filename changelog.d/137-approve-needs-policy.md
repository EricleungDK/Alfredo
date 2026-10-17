### Fixed

- `/approve N` on a task with no files/check policy is now refused (the task stays
  `Proposed`) with `Task #N has no files/check yet; set them with /permit N {...}`,
  instead of reporting `approved` and then failing `/run` with a hint (`/refresh`) that
  could not help. The `/permit` JSON error now includes an example, the `/task` receipt
  names `/permit` before `/approve`, and an older save that already holds an approval
  without a policy shows `needs policy` instead of `approved` in the task panel.
