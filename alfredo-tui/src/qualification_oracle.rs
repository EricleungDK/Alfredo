//! Fixed diagnostic parent oracle. Generated modules run only in the bounded
//! child; the parent compares JSON values without importing model-controlled code.
//! This detects incomplete/early-exit checks, not arbitrary-code attestation.
use std::collections::BTreeMap;

pub fn check_script(required_source: bool, pinned_sources: &BTreeMap<String, String>) -> String {
    let (function, inputs, expected) = if required_source {
        (
            "transform",
            serde_json::json!([-100, -1, 0, 1, 9, 100]),
            serde_json::json!([-1677, 6, 23, 40, 176, 1723]),
        )
    } else {
        (
            "parse_port",
            serde_json::json!([
                "1",
                "80",
                "00080",
                "65535",
                "",
                "0",
                "65536",
                "-1",
                "+80",
                " 80",
                "80 ",
                "8.0",
                "１２",
                "١٢",
                "9".repeat(100),
                null,
                true,
                false,
                80,
                []
            ]),
            serde_json::json!([
                1, 80, 80, 65535, null, null, null, null, null, null, null, null, null, null, null,
                null, null, null, null, null
            ]),
        )
    };
    let specification = serde_json::json!({
        "function": function, "inputs": inputs, "expected": expected, "sources": pinned_sources
    });
    // A JSON string literal also forms a valid Python string literal here: our
    // source specification contains no escapes whose meanings differ.
    let encoded =
        serde_json::to_string(&specification.to_string()).expect("fixed oracle specification");
    format!(
        r#"# Fixed Alfredo qualification parent oracle. Never import solution here.
import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import time
import hashlib

specification = json.loads({encoded})

def check_sources():
    for path, expected in specification['sources'].items():
        assert hashlib.sha256(Path(path).read_bytes()).hexdigest() == expected, path

check_sources()
# This separate process may import helpers and the model's solution. No expected
# answers are supplied to it. The parent independently checks bounded JSON values.
child_script = '''
import contextlib
import json
import os
import sys
sys.path.insert(0, os.getcwd())
request = json.loads(sys.argv[1])
with contextlib.redirect_stdout(sys.stderr):
    import solution
    function = getattr(solution, request['function'])
    results = [function(value) for value in request['inputs']]
sys.stdout.write(json.dumps(results, allow_nan=False, separators=(',', ':')))
'''
request = json.dumps({{'function': specification['function'], 'inputs': specification['inputs']}}, separators=(',', ':'))
child = subprocess.Popen(
    [sys.executable, '-I', '-B', '-c', child_script, request],
    stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    start_new_session=True, close_fds=True,
)
streams = selectors.DefaultSelector()
streams.register(child.stdout, selectors.EVENT_READ, 'stdout')
streams.register(child.stderr, selectors.EVENT_READ, 'stderr')
captured = {{'stdout': bytearray(), 'stderr': bytearray()}}
deadline = time.monotonic() + 10.0
read_complete = False
try:
    while streams.get_map():
        remaining = deadline - time.monotonic()
        assert remaining > 0, 'Fixture result child timed out'
        for key, _ in streams.select(min(remaining, 0.05)):
            chunk = os.read(key.fileobj.fileno(), 4096)
            if not chunk:
                streams.unregister(key.fileobj)
                key.fileobj.close()
                continue
            captured[key.data].extend(chunk)
            assert sum(map(len, captured.values())) <= 16384, 'Fixture result child exceeded output bound'
    read_complete = True
finally:
    streams.close()
    if read_complete:
        # Observe exit without reaping, allowing normal interpreter finalization
        # before stopping any descendants still in the owned process group.
        while time.monotonic() < deadline:
            if os.waitid(os.P_PID, child.pid, os.WEXITED | os.WNOWAIT | os.WNOHANG) is not None:
                break
            time.sleep(0.01)
    # Keep the child unreaped until the owned process group has been stopped.
    # The enclosing native check also supervises its complete sandbox lifetime.
    try:
        os.killpg(child.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    returncode = child.wait(timeout=2.0)
    for stream in [child.stdout, child.stderr]:
        stream.close()

assert returncode == 0, 'Fixture result child failed'
actual = json.loads(captured['stdout'].decode('utf-8'))
expected = specification['expected']
assert type(actual) is list and len(actual) == len(expected), 'Fixture result shape differs'
for observed, required in zip(actual, expected):
    assert type(observed) is type(required) and observed == required, 'Fixture behavior differs'
check_sources()
print('QUALIFICATION_CHECK_OK')
"#
    )
}
