use albert_mission_control::execution::{
    ControlSignal, ExecutionCallbacks, ExecutionReceipt, ExecutionRequest, RustExecutionProvider,
    StructuredFailure,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, BufRead, BufReader, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn request_cancellation(_signal: libc::c_int) {
    CANCEL_REQUESTED.store(true, Ordering::SeqCst);
}

#[cfg(unix)]
fn install_cancellation_handler() {
    unsafe {
        libc::signal(
            libc::SIGUSR1,
            request_cancellation as *const () as libc::sighandler_t,
        );
    }
}

#[cfg(not(unix))]
fn install_cancellation_handler() {}

#[derive(Serialize)]
struct ProviderResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt: Option<ExecutionReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<StructuredFailure>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderRequest {
    request: Value,
    #[serde(default)]
    control: Option<ProviderControl>,
    #[serde(default)]
    stream_events: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderControl {
    cancel_after_milliseconds: u64,
}

fn failure(code: &str, message: String) -> StructuredFailure {
    StructuredFailure {
        code: code.to_owned(),
        message,
        recoverable: true,
    }
}

fn response_for(
    input: Value,
    provider: &RustExecutionProvider,
    output: &mut dyn Write,
) -> (ProviderResponse, bool) {
    let envelope = match serde_json::from_value::<ProviderRequest>(input) {
        Ok(envelope) => envelope,
        Err(error) => {
            return (
                ProviderResponse {
                    ok: false,
                    receipt: None,
                    failure: Some(failure(
                        "contract-failure",
                        format!("execution provider envelope was invalid: {error}"),
                    )),
                },
                false,
            )
        }
    };
    let stream_events = envelope.stream_events
        || std::env::var("ALFREDO_EXECUTION_STREAM_EVENTS").as_deref() == Ok("1");
    let request = match ExecutionRequest::from_value(envelope.request) {
        Ok(request) => request,
        Err(error) => {
            return (
                ProviderResponse {
                    ok: false,
                    receipt: None,
                    failure: Some(error),
                },
                stream_events,
            )
        }
    };
    let cancellation_deadline = if let Some(control) = envelope.control {
        if control.cancel_after_milliseconds == 0 || control.cancel_after_milliseconds > 3_600_000 {
            return (
                ProviderResponse {
                    ok: false,
                    receipt: None,
                    failure: Some(failure(
                        "contract-failure",
                        "execution provider cancellation control is outside the bounded range"
                            .to_owned(),
                    )),
                },
                stream_events,
            );
        }
        Some(Instant::now() + Duration::from_millis(control.cancel_after_milliseconds))
    } else {
        None
    };
    let result = if stream_events || cancellation_deadline.is_some() {
        let mut started = |binding: albert_mission_control::execution::ProcessBinding| {
            if stream_events {
                let event = serde_json::json!({
                    "event": "process-started",
                    "process_pid": binding.pid,
                    "process_identity": binding.identity,
                });
                if serde_json::to_writer(&mut *output, &event).is_err()
                    || output.write_all(b"\n").is_err()
                    || output.flush().is_err()
                {
                    return Err(ControlSignal::Cancelled(
                        "execution provider event transport closed".to_owned(),
                    ));
                }
            }
            if CANCEL_REQUESTED.load(Ordering::SeqCst) {
                Err(ControlSignal::Cancelled(
                    "Local Agent cancellation was requested".to_owned(),
                ))
            } else {
                Ok(())
            }
        };
        let mut poll = || {
            if CANCEL_REQUESTED.load(Ordering::SeqCst) {
                Err(ControlSignal::Cancelled(
                    "Local Agent cancellation was requested".to_owned(),
                ))
            } else if cancellation_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                Err(ControlSignal::Cancelled("release cancellation".to_owned()))
            } else {
                Ok(())
            }
        };
        provider.execute_with_callbacks(
            &request,
            &mut ExecutionCallbacks {
                process_started: Some(&mut started),
                poll: Some(&mut poll),
            },
        )
    } else {
        provider.execute(&request)
    };
    (
        match result {
            Ok(receipt) => ProviderResponse {
                ok: true,
                receipt: Some(receipt),
                failure: None,
            },
            Err(error) => ProviderResponse {
                ok: false,
                receipt: None,
                failure: Some(error),
            },
        },
        stream_events,
    )
}

fn main() -> io::Result<()> {
    install_cancellation_handler();
    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let provider = RustExecutionProvider::new();
    let mut line = Vec::new();
    loop {
        CANCEL_REQUESTED.store(false, Ordering::SeqCst);
        line.clear();
        let bytes = input.read_until(b'\n', &mut line)?;
        if bytes == 0 {
            break;
        }
        if line.len() > albert_mission_control::execution::MAX_PROTOCOL_LINE_BYTES {
            let response = ProviderResponse {
                ok: false,
                receipt: None,
                failure: Some(failure(
                    "contract-failure",
                    "execution provider input line exceeds the bounded size".to_owned(),
                )),
            };
            serde_json::to_writer(&mut stdout, &response)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
            continue;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let (response, stream_events) = match serde_json::from_slice::<Value>(&line) {
            Ok(input) => response_for(input, &provider, &mut stdout),
            Err(error) => (
                ProviderResponse {
                    ok: false,
                    receipt: None,
                    failure: Some(failure(
                        "contract-failure",
                        format!("execution provider input was not valid JSON: {error}"),
                    )),
                },
                false,
            ),
        };
        if stream_events {
            let mut response = serde_json::to_value(response)?;
            response
                .as_object_mut()
                .expect("provider response serializes as an object")
                .insert("event".to_owned(), Value::String("receipt".to_owned()));
            serde_json::to_writer(&mut stdout, &response)?;
        } else {
            serde_json::to_writer(&mut stdout, &response)?;
        }
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}
