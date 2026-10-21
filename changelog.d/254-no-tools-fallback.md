### Fixed

- Chat on an Ollama model without tool support no longer fails every message with a bare
  `Ollama returned HTTP 400`. A provider failure now shows Ollama's own error text
  (about 200 characters, control characters removed). A `does not support tools` 400
  retries that round once without tools, shows one notice, and later turns with that
  model skip tools; the chat system prompt stops promising them.
