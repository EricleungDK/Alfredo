### Fixed

- `git_commit` now refuses, before asking, when the files it would commit (staged, or expanded from a directory in `files`) include `.env*`, `*.pem`, `*.key`, `*.p12`, `*.pfx` or `.git` paths; generated trees such as `vendor/` stay committable (#266).
