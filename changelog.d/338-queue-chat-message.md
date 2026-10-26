### Added

- Enter while a chat reply is running (streaming, thinking or running a tool) now queues the
  prompt instead of refusing it: it shows as `You · queued` and the prompt is cleared. Queued
  messages (up to 3) are sent in order, one turn each, when the reply completes. Esc, a failed
  reply or a restart returns them to the prompt as a draft and never sends them; an approval
  card keeps its own keys, and `/` commands are not queued (issue #338).
