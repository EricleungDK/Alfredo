### Security

- `/go` start no longer hangs when the workspace's repository blocks git (a nested gitfile
  to a FIFO, for example). Its uncommitted-changes check shares git's 15 s deadline. A check
  that times out, or that git cannot finish (a corrupt index, for example), is a start
  warning that says why, never a clean tree.
