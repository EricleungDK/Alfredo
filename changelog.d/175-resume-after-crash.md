### Fixed

- After a hard crash (kill -9, out-of-memory, power loss) mid-run, `/resume` now
  continues instead of ending the autopilot as failed with no reason. A task whose
  worker and owner lock are gone shows as `interrupted` (not `running ▶`), the footer
  says `Task #1 was running when Alfredo exited; /resume re-runs it`, and the header no
  longer counts it as failed. `/resume` applies the existing `/recover` proof, records
  the run as cancelled, requeues the task without spending `--max-repairs` and goes on to
  the integration branch; nothing is replayed. When the proof does not hold, the summary
  names the reason and a `Next:` command. `/recover ID requeue` is the explicit form.
