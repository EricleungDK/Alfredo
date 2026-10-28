### Security

- Host git calls keep reading git's error output to its end, so a repository that makes
  git print more than 4 KiB of warnings (for example `.gitattributes` with `eol=crlf` and
  many LF files) no longer kills git mid-command. Working-tree diffs and approved commits
  in such a repository work again, and only the first 4 KiB of error output is kept.
