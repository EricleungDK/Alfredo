### Security

- `/go` start no longer hangs when the workspace's repository blocks git (a nested gitfile
  to a FIFO, for example). Its uncommitted-changes check shares git's 15 s deadline, and a
  check that gets no answer is a start warning naming the timeout, never a clean tree.
