### Changed

- ADR 0004 records that chat becomes the Chat Agent, which may act on the Coding
  Workspace with owner Approval (reads auto; edits, shell, git writes ask; push always
  asks; shell sandboxed unless escalated per call). Workers and Autopilot stay isolated.
