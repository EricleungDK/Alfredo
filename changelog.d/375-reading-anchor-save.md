### Fixed

- Conversation saves no longer fail with "Invalid conversation reading anchor" while you
  are scrolled up over Ctrl+O-expanded tool results or a `/compact` summary, so turns
  are no longer lost on quit and Ctrl+Q exits 0. The anchor bound now counts those rows,
  and a scroll position the stored data cannot explain is dropped on save instead of
  refusing the snapshot (tampered files are still refused on load).
