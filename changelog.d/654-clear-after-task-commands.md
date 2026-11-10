### Fixed

- `/clear` no longer stays refused for the rest of the session after a task command (`/task`, `/permit`, `/approve`, `/run`, `/review`, `/accept`, `/go`) ran: it is refused only while a command is still in flight (running or not yet acknowledged), and the unknown-command example in the reference no longer names `/help`.
