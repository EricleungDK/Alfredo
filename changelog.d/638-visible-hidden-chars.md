### Fixed

- A worker's "SEARCH text not found" failure now shows Unicode format characters in the
  quoted line (such as U+202E and U+200B) as `<U+XXXX>`, and the "file is too large"
  message, in a task failure and in the plan size check, shows the file path the same
  way, cut to 200 characters on one line. U+0890 and U+0891 are now treated as hidden
  characters everywhere: they show as `<U+0890>` / `<U+0891>`, a plan containing them
  is refused, and project notes drop them. Which worker answers and files are accepted
  is otherwise unchanged.
