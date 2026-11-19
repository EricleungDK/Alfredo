### Fixed

- Pasting text with bare carriage-return line breaks (old Mac, some terminals) into the
  prompt or the selector no longer glues the lines together; each `\r` or `\r\n` becomes
  one newline.
