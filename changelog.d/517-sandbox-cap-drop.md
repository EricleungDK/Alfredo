### Security

- The Bubblewrap sandbox now drops every Linux capability (`--cap-drop ALL`) for worker
  checks, chat `run_command`, the run boundary and the eval grader. Run as root, Alfredo
  used to leave CAP_SYS_ADMIN inside the sandbox, so a sandboxed command could remount a
  read-only bind such as the workspace `.git` read-write and write through it to the host.
- The prepared-argv validator now refuses any sandbox argv that does not contain
  `--cap-drop ALL`.
