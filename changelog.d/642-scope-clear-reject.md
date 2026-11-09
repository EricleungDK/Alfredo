### Fixed

- `/scope-clear` with no Wayfinder flow now keeps its text as a rejected command like every
  other refusal: the next character you type replaces it instead of being appended
  (`Unknown command /scope-clear/foo`). Left and Backspace still keep it for editing.
