### Security

- The chat tools, `@path` mentions, `git_commit` and planner context refuse more common
  credential files: `_netrc`, `.my.cnf`, Composer's `auth.json`, `.m2/settings.xml`,
  rclone and gcloud configs, KeePass databases, `.bash_history`, `gradle.properties` and
  similar names. Matching ignores case, and `.` and empty path components are dropped
  before matching, so `.docker//config.json` is refused too (#523).
