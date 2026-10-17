### Fixed

- `/model NAME` now works on a fresh launch without opening `/models` first: it shows
  `Checking installed models…`, then selects the model once discovery finishes. A name
  that is not installed reports `Model NAME is not installed; installed: …`, and a
  failed discovery says it could not check installed models instead of `Model not in
  the catalog`.
