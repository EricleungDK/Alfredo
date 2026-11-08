### Changed

- While `/go` runs, the task detail and the agent view no longer stream every file the
  worker writes: each FILE or EDIT block, and each fenced code snippet in the model's
  prose, is folded to one row such as `▸ calc.py · 42 lines` (`… so far` while it is
  still streaming). The model's prose stays visible. Ctrl+O shows the code under each
  heading and folds it again; in the agent view it also shows the full instruction text.
  A live output tail whose start was dropped folds the lines before its first file into
  `… N earlier lines`.
