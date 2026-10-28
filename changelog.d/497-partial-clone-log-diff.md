### Security

- `git_log`, `git_branch` and `git_diff` with a base and target now refuse a partial clone
  (`extensions.partialclone` or a `remote.*.promisor` remote) before git runs, naming the
  key and the transport program it would run (`remote.<name>.uploadpack`). Until now only
  the host git environment (`GIT_NO_LAZY_FETCH`) stopped a lazy fetch there, which a git
  without that support does not honour. Repositories that are not partial clones are
  unaffected.
