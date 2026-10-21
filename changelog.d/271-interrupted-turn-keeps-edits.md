### Fixed

- A chat tool call now shows as a `⚙` row as soon as it finishes, so a turn cancelled
  or failed later in the same round still shows the edit it applied, and the next
  request after an interrupted turn keeps that turn's prompt and a short note of the
  changes already applied, so the model no longer redoes or contradicts them.
