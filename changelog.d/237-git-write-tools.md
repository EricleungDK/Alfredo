### Added

- Chat can commit (`git_commit`: staged changes or named files), switch or create
  branches (`git_switch`), merge a branch such as `alfredo/go-12` into the current one
  (`git_merge`) and push (`git_push`) on your real repository. Each call shows an
  approval card with its effect (files and message, from → to, commits and files to
  merge or publish); push cards are red and are asked every time. Merge conflicts are
  listed and the merge is aborted, leaving the repository as it was. Hooks never run;
  option-like refs, unknown remotes and a repository changed after the preview are refused.
