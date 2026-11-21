### Fixed

- Ctrl+R after a failed or cancelled chat turn that had already applied changes (an
  approved write, edit, git action or a killed `run_command`) no longer makes the model
  redo them: the retried prompt now ends with a framed note naming what the earlier
  attempt applied ("Do not repeat these; continue from there"). The first attempt's `⚙`
  rows stay in the transcript under `· earlier attempt`, and the note survives a
  restart. A turn that applied nothing is retried exactly as before.
