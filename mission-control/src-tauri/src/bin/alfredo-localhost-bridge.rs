use albert_mission_control::{BridgeFailure, WorkstationBridge};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::process::ExitCode;
use std::sync::{
    mpsc::{self, Receiver, SyncSender, TrySendError},
    Arc, Mutex,
};
use std::thread;

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_REQUEST_ID_BYTES: usize = 256;
const CONTROL_WORKER_COUNT: usize = 4;
const CONTROL_QUEUE_CAPACITY: usize = 32;
const RUNNER_QUEUE_CAPACITY: usize = 1;
const RESPONSE_QUEUE_CAPACITY: usize = 32;

#[cfg(unix)]
mod unix_process_group {
    use std::os::raw::c_int;

    const SIGKILL: c_int = 9;

    unsafe extern "C" {
        fn getpid() -> c_int;
        fn getppid() -> c_int;
        fn getpgrp() -> c_int;
        #[cfg(test)]
        fn getpgid(pid: c_int) -> c_int;
        fn kill(pid: c_int, signal: c_int) -> c_int;
    }

    #[derive(Clone, Copy, Debug)]
    pub(super) struct OwnedProcessGroup {
        id: c_int,
    }

    impl OwnedProcessGroup {
        pub(super) fn capture_if_dedicated() -> Option<Self> {
            // SAFETY: These process identity functions have no preconditions.
            let (process, parent, group) = unsafe { (getpid(), getppid(), getpgrp()) };
            // The localhost gateway creates a detached group led by Cargo. Cargo
            // may either exec the bridge (process == group) or remain its parent
            // (parent == group). Refuse to arm for a shared shell/test group.
            (group > 1 && (group == process || group == parent)).then_some(Self { id: group })
        }

        pub(super) fn terminate_now(self) -> ! {
            // SAFETY: A negative PID targets exactly the captured process group.
            // SIGKILL is required because no owner remains to deliver a fallback
            // signal if an isolated runner ignores graceful termination.
            let result = unsafe { kill(-self.id, SIGKILL) };
            if result != 0 {
                eprintln!(
                    "Alfredo localhost bridge could not terminate orphaned process group {}: {}",
                    self.id,
                    std::io::Error::last_os_error()
                );
            }
            // Successful group delivery includes this bridge, so execution will
            // not normally reach here. Never wait for workers if delivery failed.
            std::process::abort()
        }
    }

    #[cfg(test)]
    pub(super) fn current_group() -> c_int {
        // SAFETY: getpgrp has no preconditions.
        unsafe { getpgrp() }
    }

    #[cfg(test)]
    pub(super) fn group_of(process: c_int) -> Option<c_int> {
        // SAFETY: getpgid accepts any process identifier; failure is reported as -1.
        let group = unsafe { getpgid(process) };
        (group >= 0).then_some(group)
    }

    #[cfg(test)]
    pub(super) fn process_exists(process: c_int) -> bool {
        // SAFETY: Signal zero performs existence/permission checking only.
        unsafe { kill(process, 0) == 0 }
    }

    #[cfg(test)]
    pub(super) fn group_exists(group: c_int) -> bool {
        group > 1 && {
            // SAFETY: Signal zero performs existence/permission checking only.
            unsafe { kill(-group, 0) == 0 }
        }
    }

    #[cfg(test)]
    pub(super) fn force_terminate_group(group: c_int) {
        if group > 1 {
            // SAFETY: Test callers first verify this is their spawned child's
            // dedicated group, never the Cargo test runner's process group.
            let _ = unsafe { kill(-group, SIGKILL) };
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum OwnerLossBoundary {
    Graceful,
    #[cfg(unix)]
    TerminateProcessGroup(unix_process_group::OwnedProcessGroup),
}

impl OwnerLossBoundary {
    fn for_localhost_process() -> Self {
        #[cfg(unix)]
        if let Some(group) = unix_process_group::OwnedProcessGroup::capture_if_dedicated() {
            return Self::TerminateProcessGroup(group);
        }
        Self::Graceful
    }

    fn stdin_closed(self) {
        match self {
            Self::Graceful => {}
            #[cfg(unix)]
            Self::TerminateProcessGroup(group) => group.terminate_now(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BridgeRequest {
    id: String,
    command: String,
    args: Map<String, Value>,
}

#[derive(Debug, Serialize)]
struct BridgeResponse {
    id: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<BridgeFailure>,
}

impl BridgeResponse {
    fn success(id: String, value: Value) -> Self {
        Self {
            id,
            ok: true,
            value: Some(value),
            error: None,
        }
    }

    fn failure(id: String, error: BridgeFailure) -> Self {
        Self {
            id,
            ok: false,
            value: None,
            error: Some(error),
        }
    }
}

enum BoundedLine {
    Eof,
    Line(Vec<u8>),
    TooLarge,
}

fn contract_failure(message: impl Into<String>) -> BridgeFailure {
    BridgeFailure {
        code: "contract-failure".to_owned(),
        message: message.into(),
        recoverable: false,
    }
}

fn capacity_failure(lane: &str) -> BridgeFailure {
    BridgeFailure {
        code: "bridge-capacity-exceeded".to_owned(),
        message: format!("The bounded localhost bridge {lane} lane is full; retry the request."),
        recoverable: true,
    }
}

fn read_bounded_line<R: BufRead>(reader: &mut R) -> io::Result<BoundedLine> {
    let mut line = Vec::new();
    let mut saw_bytes = false;
    let mut exceeded = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if !saw_bytes {
                Ok(BoundedLine::Eof)
            } else if exceeded {
                Ok(BoundedLine::TooLarge)
            } else {
                Ok(BoundedLine::Line(line))
            };
        }
        saw_bytes = true;
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        let payload_bytes = newline.map_or(consumed, |index| index);
        if !exceeded {
            if line.len().saturating_add(payload_bytes) > MAX_REQUEST_BYTES {
                exceeded = true;
                line.clear();
            } else {
                line.extend_from_slice(&available[..payload_bytes]);
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            if exceeded {
                return Ok(BoundedLine::TooLarge);
            }
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(BoundedLine::Line(line));
        }
    }
}

fn request_id_hint(line: &[u8]) -> String {
    serde_json::from_slice::<Value>(line)
        .ok()
        .and_then(|value| value.get("id")?.as_str().map(str::to_owned))
        .filter(|id| !id.is_empty() && id.len() <= MAX_REQUEST_ID_BYTES)
        .unwrap_or_default()
}

fn parse_request(line: &[u8]) -> Result<BridgeRequest, BridgeResponse> {
    let id_hint = request_id_hint(line);
    let request: BridgeRequest = match serde_json::from_slice(line) {
        Ok(request) => request,
        Err(error) => {
            return Err(BridgeResponse::failure(
                id_hint,
                contract_failure(format!("Invalid localhost bridge request: {error}")),
            ))
        }
    };
    if request.id.is_empty() || request.id.len() > MAX_REQUEST_ID_BYTES {
        return Err(BridgeResponse::failure(
            String::new(),
            contract_failure("Localhost bridge request id must be 1 to 256 bytes."),
        ));
    }
    Ok(request)
}

fn dispatch_request<D>(dispatcher: &D, request: BridgeRequest) -> BridgeResponse
where
    D: Fn(&str, Value) -> Result<Value, BridgeFailure>,
{
    let id = request.id;
    match dispatcher(&request.command, Value::Object(request.args)) {
        Ok(value) => BridgeResponse::success(id, value),
        Err(error) => BridgeResponse::failure(id, error),
    }
}

fn write_response<W: Write>(output: &mut W, response: &BridgeResponse) -> io::Result<()> {
    serde_json::to_writer(&mut *output, response)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    output.write_all(b"\n")?;
    output.flush()
}

fn worker_loop<D>(
    receiver: Arc<Mutex<Receiver<BridgeRequest>>>,
    responses: SyncSender<BridgeResponse>,
    dispatcher: Arc<D>,
) where
    D: Fn(&str, Value) -> Result<Value, BridgeFailure> + Send + Sync,
{
    loop {
        let request = receiver
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .recv();
        let Ok(request) = request else {
            return;
        };
        if responses
            .send(dispatch_request(dispatcher.as_ref(), request))
            .is_err()
        {
            return;
        }
    }
}

fn writer_loop<W: Write>(output: &mut W, responses: Receiver<BridgeResponse>) -> io::Result<()> {
    for response in responses {
        write_response(output, &response)?;
    }
    Ok(())
}

fn channel_closed(lane: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        format!("The localhost bridge {lane} lane stopped unexpectedly."),
    )
}

fn enqueue_request(
    sender: &SyncSender<BridgeRequest>,
    responses: &SyncSender<BridgeResponse>,
    request: BridgeRequest,
    lane: &str,
) -> io::Result<()> {
    match sender.try_send(request) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(request)) => responses
            .send(BridgeResponse::failure(request.id, capacity_failure(lane)))
            .map_err(|_| channel_closed("response")),
        Err(TrySendError::Disconnected(_)) => Err(channel_closed(lane)),
    }
}

fn accept_requests<R: BufRead>(
    input: &mut R,
    control_sender: &SyncSender<BridgeRequest>,
    runner_sender: &SyncSender<BridgeRequest>,
    responses: &SyncSender<BridgeResponse>,
    owner_loss: OwnerLossBoundary,
) -> io::Result<()> {
    loop {
        let request = match read_bounded_line(input)? {
            BoundedLine::Eof => {
                owner_loss.stdin_closed();
                return Ok(());
            }
            BoundedLine::Line(line) => match parse_request(&line) {
                Ok(request) => request,
                Err(response) => {
                    responses
                        .send(response)
                        .map_err(|_| channel_closed("response"))?;
                    continue;
                }
            },
            BoundedLine::TooLarge => {
                responses
                    .send(BridgeResponse::failure(
                        String::new(),
                        contract_failure(format!(
                            "Localhost bridge request exceeds {MAX_REQUEST_BYTES} bytes."
                        )),
                    ))
                    .map_err(|_| channel_closed("response"))?;
                continue;
            }
        };
        if request.command == "workstation_session_run" {
            enqueue_request(runner_sender, responses, request, "runner")?;
        } else {
            enqueue_request(control_sender, responses, request, "control")?;
        }
    }
}

fn join_failure(thread_name: &str) -> io::Error {
    io::Error::other(format!(
        "The localhost bridge {thread_name} thread panicked."
    ))
}

#[cfg(test)]
fn serve_with_dispatcher<R, W, D>(
    dispatcher: Arc<D>,
    input: &mut R,
    output: &mut W,
) -> io::Result<()>
where
    R: BufRead,
    W: Write + Send,
    D: Fn(&str, Value) -> Result<Value, BridgeFailure> + Send + Sync,
{
    serve_with_owner_loss(dispatcher, input, output, OwnerLossBoundary::Graceful)
}

fn serve_with_owner_loss<R, W, D>(
    dispatcher: Arc<D>,
    input: &mut R,
    output: &mut W,
    owner_loss: OwnerLossBoundary,
) -> io::Result<()>
where
    R: BufRead,
    W: Write + Send,
    D: Fn(&str, Value) -> Result<Value, BridgeFailure> + Send + Sync,
{
    thread::scope(|scope| {
        let (responses, response_receiver) = mpsc::sync_channel(RESPONSE_QUEUE_CAPACITY);
        let (controls, control_receiver) = mpsc::sync_channel(CONTROL_QUEUE_CAPACITY);
        let (runners, runner_receiver) = mpsc::sync_channel(RUNNER_QUEUE_CAPACITY);
        let control_receiver = Arc::new(Mutex::new(control_receiver));
        let runner_receiver = Arc::new(Mutex::new(runner_receiver));

        let writer = scope.spawn(move || writer_loop(output, response_receiver));
        let mut workers = Vec::with_capacity(CONTROL_WORKER_COUNT + 1);
        for _ in 0..CONTROL_WORKER_COUNT {
            let receiver = Arc::clone(&control_receiver);
            let worker_responses = responses.clone();
            let worker_dispatcher = Arc::clone(&dispatcher);
            workers.push(
                scope.spawn(move || worker_loop(receiver, worker_responses, worker_dispatcher)),
            );
        }
        {
            let receiver = Arc::clone(&runner_receiver);
            let worker_responses = responses.clone();
            let worker_dispatcher = Arc::clone(&dispatcher);
            workers.push(
                scope.spawn(move || worker_loop(receiver, worker_responses, worker_dispatcher)),
            );
        }

        let accept_result = accept_requests(input, &controls, &runners, &responses, owner_loss);
        drop(controls);
        drop(runners);
        drop(responses);

        let mut worker_result = Ok(());
        for worker in workers {
            if worker.join().is_err() && worker_result.is_ok() {
                worker_result = Err(join_failure("worker"));
            }
        }
        let writer_result = writer
            .join()
            .map_err(|_| join_failure("writer"))
            .and_then(|result| result);

        accept_result?;
        worker_result?;
        writer_result
    })
}

#[cfg(test)]
fn serve<R: BufRead, W: Write + Send>(
    bridge: Arc<WorkstationBridge>,
    input: &mut R,
    output: &mut W,
) -> io::Result<()> {
    let dispatcher = Arc::new(move |command: &str, args: Value| bridge.dispatch(command, args));
    serve_with_dispatcher(dispatcher, input, output)
}

fn serve_localhost_process<R: BufRead, W: Write + Send>(
    bridge: Arc<WorkstationBridge>,
    input: &mut R,
    output: &mut W,
) -> io::Result<()> {
    let dispatcher = Arc::new(move |command: &str, args: Value| bridge.dispatch(command, args));
    serve_with_owner_loss(
        dispatcher,
        input,
        output,
        OwnerLossBoundary::for_localhost_process(),
    )
}

fn main() -> ExitCode {
    let bridge = Arc::new(WorkstationBridge::from_environment());
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = BufReader::new(stdin.lock());
    let mut output = BufWriter::new(stdout);
    match serve_localhost_process(bridge, &mut input, &mut output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Alfredo localhost bridge I/O failure: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Barrier, Condvar,
    };

    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;
    #[cfg(unix)]
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::process::{Child, Command, Stdio};
    #[cfg(unix)]
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    #[derive(Default)]
    struct SequenceState {
        runner_started: bool,
        response_written: bool,
    }

    struct SignalingWriter {
        bytes: Vec<u8>,
        signal: Arc<(Mutex<SequenceState>, Condvar)>,
    }

    impl Write for SignalingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            if bytes.contains(&b'\n') {
                let (state, condition) = self.signal.as_ref();
                state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .response_written = true;
                condition.notify_all();
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[cfg(unix)]
    struct ProcessGroupGuard {
        id: i32,
        armed: bool,
    }

    #[cfg(unix)]
    impl Drop for ProcessGroupGuard {
        fn drop(&mut self) {
            if self.armed {
                unix_process_group::force_terminate_group(self.id);
            }
        }
    }

    #[cfg(unix)]
    struct TempFileGuard(PathBuf);

    #[cfg(unix)]
    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[cfg(unix)]
    fn wait_for_child(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = child.try_wait().expect("fixture status should be readable") {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if predicate() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn jsonl_loop_returns_exact_success_and_failure_envelopes() {
        let bridge = Arc::new(WorkstationBridge::from_environment());
        let requests = concat!(
            "{\"id\":\"mark-1\",\"command\":\"performance_mark\",\"args\":{\"request\":{\"stage\":\"S3\",\"boundary\":\"start\",\"clock\":\"frontend\"}}}\n",
            "{\"id\":\"bad-1\",\"command\":\"python_argv\",\"args\":{\"argv\":[\"workspace-snapshot\"]}}\n",
        );
        let mut input = Cursor::new(requests.as_bytes());
        let mut output = Vec::new();

        serve(bridge, &mut input, &mut output).expect("JSONL loop should complete");

        let responses = String::from_utf8(output).expect("responses should be UTF-8");
        let decoded = responses
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("response should be JSON"))
            .collect::<Vec<_>>();
        assert_eq!(decoded.len(), 2);
        let mark = decoded
            .iter()
            .find(|response| response["id"] == "mark-1")
            .expect("performance response should be present");
        let bad = decoded
            .iter()
            .find(|response| response["id"] == "bad-1")
            .expect("failure response should be present");
        assert_eq!(
            *mark,
            serde_json::json!({"id": "mark-1", "ok": true, "value": {"recorded": false}})
        );
        assert_eq!(bad["ok"], false);
        assert_eq!(bad["error"]["code"], "contract-failure");
        assert!(bad.get("value").is_none());
    }

    #[test]
    fn malformed_request_preserves_a_valid_correlation_id_and_loop_continues() {
        let bridge = Arc::new(WorkstationBridge::from_environment());
        let requests = concat!(
            "{\"id\":\"malformed-1\",\"command\":\"performance_mark\",\"args\":[]}\n",
            "{\"id\":\"mark-2\",\"command\":\"performance_mark\",\"args\":{\"request\":{\"stage\":\"S3\",\"boundary\":\"start\",\"clock\":\"frontend\"}}}\n",
        );
        let mut input = Cursor::new(requests.as_bytes());
        let mut output = Vec::new();

        serve(bridge, &mut input, &mut output).expect("JSONL loop should recover");

        let decoded = String::from_utf8(output)
            .expect("responses should be UTF-8")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("response should be JSON"))
            .collect::<Vec<_>>();
        let malformed = decoded
            .iter()
            .find(|response| response["id"] == "malformed-1")
            .expect("malformed response should be present");
        let mark = decoded
            .iter()
            .find(|response| response["id"] == "mark-2")
            .expect("performance response should be present");
        assert_eq!(malformed["ok"], false);
        assert_eq!(mark["ok"], true);
    }

    #[test]
    fn oversized_line_is_drained_without_desynchronizing_the_next_request() {
        let mut bytes = vec![b'x'; MAX_REQUEST_BYTES + 1];
        bytes.extend_from_slice(b"\n{}\n");
        let mut input = Cursor::new(bytes);

        assert!(matches!(
            read_bounded_line(&mut input).expect("oversized line should be read"),
            BoundedLine::TooLarge
        ));
        match read_bounded_line(&mut input).expect("next line should remain readable") {
            BoundedLine::Line(line) => assert_eq!(line, b"{}"),
            _ => panic!("expected a bounded second line"),
        }
    }

    #[test]
    fn later_fast_control_response_precedes_an_earlier_blocked_runner() {
        let signal = Arc::new((Mutex::new(SequenceState::default()), Condvar::new()));
        let dispatcher_signal = Arc::clone(&signal);
        let dispatcher = Arc::new(move |command: &str, _args: Value| {
            let (state, condition) = dispatcher_signal.as_ref();
            if command == "workstation_session_run" {
                let mut state = state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state.runner_started = true;
                condition.notify_all();
                while !state.response_written {
                    state = condition
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                Ok(serde_json::json!({"kind": "runner"}))
            } else {
                let mut state = state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                while !state.runner_started {
                    state = condition
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                Ok(serde_json::json!({"kind": "control"}))
            }
        });
        let requests = concat!(
            "{\"id\":\"runner-1\",\"command\":\"workstation_session_run\",\"args\":{}}\n",
            "{\"id\":\"control-1\",\"command\":\"performance_mark\",\"args\":{}}\n",
        );
        let mut input = Cursor::new(requests.as_bytes());
        let mut output = SignalingWriter {
            bytes: Vec::new(),
            signal,
        };

        serve_with_dispatcher(dispatcher, &mut input, &mut output)
            .expect("control lane should remain responsive while runner is blocked");

        let ids = String::from_utf8(output.bytes)
            .expect("responses should be UTF-8")
            .lines()
            .map(|line| {
                serde_json::from_str::<Value>(line).expect("response should be JSON")["id"]
                    .as_str()
                    .expect("response should have an id")
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, ["control-1", "runner-1"]);
    }

    #[test]
    fn fixed_worker_pools_hold_runner_and_control_concurrency_bounds() {
        let active_runners = Arc::new(AtomicUsize::new(0));
        let maximum_runners = Arc::new(AtomicUsize::new(0));
        let active_controls = Arc::new(AtomicUsize::new(0));
        let maximum_controls = Arc::new(AtomicUsize::new(0));
        let control_barrier = Arc::new(Barrier::new(CONTROL_WORKER_COUNT));
        let dispatcher = {
            let active_runners = Arc::clone(&active_runners);
            let maximum_runners = Arc::clone(&maximum_runners);
            let active_controls = Arc::clone(&active_controls);
            let maximum_controls = Arc::clone(&maximum_controls);
            let control_barrier = Arc::clone(&control_barrier);
            Arc::new(move |command: &str, _args: Value| {
                if command == "workstation_session_run" {
                    let active = active_runners.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum_runners.fetch_max(active, Ordering::SeqCst);
                    thread::yield_now();
                    active_runners.fetch_sub(1, Ordering::SeqCst);
                } else {
                    let active = active_controls.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum_controls.fetch_max(active, Ordering::SeqCst);
                    control_barrier.wait();
                    active_controls.fetch_sub(1, Ordering::SeqCst);
                }
                Ok(Value::Null)
            })
        };
        let mut requests = String::new();
        for index in 0..3 {
            requests.push_str(&format!(
                "{{\"id\":\"runner-{index}\",\"command\":\"workstation_session_run\",\"args\":{{}}}}\n"
            ));
        }
        for index in 0..CONTROL_WORKER_COUNT {
            requests.push_str(&format!(
                "{{\"id\":\"control-{index}\",\"command\":\"performance_mark\",\"args\":{{}}}}\n"
            ));
        }
        let mut input = Cursor::new(requests.into_bytes());
        let mut output = Vec::new();

        serve_with_dispatcher(dispatcher, &mut input, &mut output)
            .expect("bounded worker pools should drain");

        assert_eq!(maximum_runners.load(Ordering::SeqCst), 1);
        assert_eq!(
            maximum_controls.load(Ordering::SeqCst),
            CONTROL_WORKER_COUNT
        );
        assert_eq!(active_runners.load(Ordering::SeqCst), 0);
        assert_eq!(active_controls.load(Ordering::SeqCst), 0);
        assert_eq!(String::from_utf8(output).unwrap().lines().count(), 7);
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "subprocess fixture invoked by owner_death_kills_the_actual_process_group"]
    fn owner_death_process_group_fixture() {
        assert_eq!(
            std::env::var("ALFREDO_OWNER_DEATH_FIXTURE").as_deref(),
            Ok("1")
        );
        let pid_file = PathBuf::from(
            std::env::var_os("ALFREDO_OWNER_DEATH_PID_FILE")
                .expect("fixture descendant PID file should be configured"),
        );
        let dispatcher = Arc::new(move |command: &str, _args: Value| {
            assert_eq!(command, "workstation_session_run");
            let mut descendant = Command::new("/bin/sleep")
                .arg("120")
                .spawn()
                .expect("fixture descendant should start");
            fs::write(&pid_file, descendant.id().to_string())
                .expect("fixture descendant PID should be published");
            let status = descendant
                .wait()
                .expect("fixture descendant should remain waitable");
            Ok(serde_json::json!({"success": status.success()}))
        });
        let stdin = io::stdin();
        let mut input = BufReader::new(stdin.lock());
        let mut output = io::sink();

        serve_with_owner_loss(
            dispatcher,
            &mut input,
            &mut output,
            OwnerLossBoundary::for_localhost_process(),
        )
        .expect("fixture bridge should serve until its owner closes stdin");
        panic!("owner loss should terminate the fixture process group");
    }

    #[cfg(unix)]
    #[test]
    fn owner_death_kills_the_actual_process_group() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should follow the Unix epoch")
            .as_nanos();
        let pid_file = std::env::temp_dir().join(format!(
            "alfredo-owner-death-{}-{unique}.pid",
            std::process::id()
        ));
        let _pid_file_guard = TempFileGuard(pid_file.clone());
        let executable = std::env::current_exe().expect("test executable should resolve");
        let mut child = Command::new(executable)
            .arg("--exact")
            .arg("tests::owner_death_process_group_fixture")
            .arg("--ignored")
            .arg("--nocapture")
            .env("ALFREDO_OWNER_DEATH_FIXTURE", "1")
            .env("ALFREDO_OWNER_DEATH_PID_FILE", &pid_file)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .expect("owner-death fixture should start");
        let group_id = i32::try_from(child.id()).expect("fixture PID should fit pid_t");
        if unix_process_group::group_of(group_id) != Some(group_id)
            || unix_process_group::current_group() == group_id
        {
            let _ = child.kill();
            let _ = child.wait();
            panic!("fixture was not isolated from the Cargo test process group");
        }
        let mut group_guard = ProcessGroupGuard {
            id: group_id,
            armed: true,
        };
        let fixture_input = child.stdin.as_mut().expect("fixture stdin should be piped");
        writeln!(
            fixture_input,
            "{{\"id\":\"runner-1\",\"command\":\"workstation_session_run\",\"args\":{{}}}}"
        )
        .expect("runner request should be written");
        fixture_input.flush().expect("runner request should flush");

        let descendant_ready = wait_until(Duration::from_secs(5), || pid_file.is_file());
        if !descendant_ready {
            let fixture_status = child.try_wait().expect("fixture status should be readable");
            panic!("fixture descendant did not start; fixture status: {fixture_status:?}");
        }
        let descendant_id = fs::read_to_string(&pid_file)
            .expect("fixture descendant PID should be readable")
            .parse::<i32>()
            .expect("fixture descendant PID should be numeric");
        assert_eq!(
            unix_process_group::group_of(descendant_id),
            Some(group_id),
            "fixture descendant must inherit only the isolated fixture group"
        );

        drop(child.stdin.take());
        let status = wait_for_child(&mut child, Duration::from_secs(5));
        if status.is_none() {
            unix_process_group::force_terminate_group(group_id);
            let _ = child.wait();
            panic!("fixture did not terminate promptly after bridge stdin owner loss");
        }
        assert!(
            !status.expect("status checked above").success(),
            "owner-death termination must not look like a graceful bridge exit"
        );
        assert!(
            wait_until(Duration::from_secs(5), || {
                !unix_process_group::process_exists(descendant_id)
                    && !unix_process_group::group_exists(group_id)
            }),
            "the isolated runner descendant or fixture process group survived owner loss"
        );
        group_guard.armed = false;
    }
}
