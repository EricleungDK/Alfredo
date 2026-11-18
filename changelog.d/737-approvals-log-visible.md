### Security

- The chat approvals log (F4) now records what ran, visibly. A newline in an approved
  command is shown as `↵` instead of being deleted (which merged its lines), a bidi,
  zero-width or tag character is shown as `<U+XXXX>` instead of drawn raw, and a
  command longer than 200 characters ends with `…` instead of being cut silently. Records
  already on disk are shown the same way (issue #737).
