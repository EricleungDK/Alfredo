### Security

- Read-tier `git_status` and `git_diff` pass `--ignore-submodules=all` on the command line, so a repository's `submodule.<name>.ignore=none` can no longer make git run inside a submodule and apply that submodule's own filter drivers or config includes.
