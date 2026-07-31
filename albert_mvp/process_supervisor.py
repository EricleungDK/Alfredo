"""Supervise one bounded payload inside its private PID namespace.

The supervisor remains namespace PID 1 after the payload leader exits so that
orphaned descendants stay observable.  It gives those descendants a short
grace period to finish and then exits, which makes the kernel tear down every
remaining process in the namespace, including children that called ``setsid``
or scrubbed their environment.
"""

from __future__ import annotations

import ctypes
import os
import subprocess
import sys
import time


_PR_SET_CHILD_SUBREAPER = 36


def _become_subreaper() -> None:
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(_PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))


def _wait_for_descendants(grace_seconds: float) -> bool:
    """Return true when descendants drained, false when grace expired."""

    deadline = time.monotonic() + grace_seconds
    while True:
        try:
            child_pid, _status = os.waitpid(-1, os.WNOHANG)
        except ChildProcessError:
            return True
        if child_pid > 0:
            continue
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return False
        time.sleep(min(0.01, remaining))


def main(argv: list[str]) -> int:
    if len(argv) < 3 or argv[1] != "--":
        print(
            "Usage: process_supervisor.py GRACE_SECONDS -- COMMAND [ARG ...]",
            file=sys.stderr,
        )
        return 126
    try:
        grace_seconds = float(argv[0])
    except ValueError:
        print("Invalid descendant grace period.", file=sys.stderr)
        return 126
    if grace_seconds < 0:
        print("Invalid descendant grace period.", file=sys.stderr)
        return 126
    try:
        _become_subreaper()
        payload = subprocess.Popen(argv[2:])
        payload_status = payload.wait()
    except (OSError, ValueError) as error:
        print(f"Unable to supervise bounded process: {error}", file=sys.stderr)
        return 126

    if not _wait_for_descendants(grace_seconds):
        print(
            "Process descendants timed out "
            f"{grace_seconds} seconds after the leader exited and were terminated.",
            file=sys.stderr,
        )
        return 124
    return payload_status if payload_status >= 0 else 128 - payload_status


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
