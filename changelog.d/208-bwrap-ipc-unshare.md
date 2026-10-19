### Security

- Worker and check sandboxes now also unshare the IPC, UTS and cgroup namespaces
  (`--unshare-ipc --unshare-uts --unshare-cgroup-try`), so a sandboxed command can no
  longer reach host SysV shared memory, semaphores or message queues.
- The Bubblewrap argv validator now refuses any prepared argv that does not contain
  `--unshare-net` and `--unshare-ipc` exactly once, so the network and IPC isolation
  cannot be dropped silently.
