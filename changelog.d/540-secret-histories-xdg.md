### Security

- The secret-file list now also refuses `.zsh_history`, `.zhistory`, `.mysql_history`,
  `.psql_history`, `.python_history`, `.node_repl_history`, `.sqlite_history`, `.authinfo`,
  `.sentryclirc`, `.config/git/credentials`, `.config/pip/pip.conf` and `.config/netrc`
  for every chat tool, `@path` and planner context. Near-miss names such as
  `docs/history.md` and `config/git/credentials-guide.md` stay readable.
