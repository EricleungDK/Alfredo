from __future__ import annotations

from contextlib import redirect_stderr, redirect_stdout
import io
import json
import sys
from typing import TextIO

from .cli import main


def serve(source: TextIO = sys.stdin, destination: TextIO = sys.stdout) -> None:
    """Serve correlated CLI requests over newline-delimited JSON."""
    for line in source:
        if not line.strip():
            continue
        request_id = ""
        stdout = io.StringIO()
        stderr = io.StringIO()
        try:
            request = json.loads(line)
            request_id = request["id"]
            argv = request["argv"]
            if not isinstance(request_id, str) or not isinstance(argv, list):
                raise ValueError("id must be a string and argv must be a list")
            with redirect_stdout(stdout), redirect_stderr(stderr):
                exit_code = main([str(value) for value in argv])
            response = {
                "id": request_id,
                "success": exit_code == 0,
                "stdout": stdout.getvalue(),
                "stderr": stderr.getvalue(),
            }
        except (KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
            response = {
                "id": request_id,
                "success": False,
                "stdout": "",
                "stderr": json.dumps({"error": {"code": "contract-failure", "message": str(error), "recoverable": True}}),
            }
        destination.write(json.dumps(response, sort_keys=True) + "\n")
        destination.flush()


if __name__ == "__main__":
    serve()
