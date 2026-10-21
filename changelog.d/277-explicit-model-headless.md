### Fixed

- An explicit `--model NAME` now applies to headless `--go`, headless `--chat` and
  `--doctor` even when the conversation already has a saved model. It is checked
  against the installed list before the conversation is opened (exit 2 names it), so a
  mistyped `--model` is no longer saved and a corrected one recovers; the run never
  overwrites the saved model. The headless JSON gains `"model"` and stderr prints
  `model NAME`. The terminal still keeps a restored conversation's model.
