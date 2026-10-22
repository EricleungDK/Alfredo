### Security

- Headless chat no longer prints control characters from model-chosen text on stderr.
  An approval line such as `approval new file …: approved` could carry escape sequences
  that rewrite a terminal or a CI log; those characters are now removed and line breaks
  become spaces.
- `write_file` and `edit_file` refuse a path that contains a control character (such as an
  escape or bell character) before asking for approval, so no file with such a name is
  created or edited.
