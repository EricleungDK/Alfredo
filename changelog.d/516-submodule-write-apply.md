### Security

- Approved chat `git_merge`, `git_switch` to an existing branch and `git_commit` of named
  files that reach a submodule are refused before the card when `.gitmodules` or git config
  sets a `submodule.<name>.ignore` other than `all`. Such an override made git open the
  submodule and run the clean filters (or block on an `include.path` FIFO) in its own
  `.git/modules/<name>/config` after approval (#516).
