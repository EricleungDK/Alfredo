use albert_mission_control::execution::{
    ExecutionReceipt, ExecutionRequest, RustExecutionProvider, StructuredFailure,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, BufRead, BufReader, Write};

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
}

fn failure(code: &str, message: String) -> StructuredFailure {
    StructuredFailure {
        code: code.to_owned(),
        message,
        recoverable: true,
    }
}

fn response_for(input: Value, provider: &RustExecutionProvider) -> ProviderResponse {
    let request_value = match serde_json::from_value::<ProviderRequest>(input) {
        Ok(envelope) => envelope.request,
        Err(error) => {
            return ProviderResponse {
                ok: false,
                receipt: None,
                failure: Some(failure(
                    "contract-failure",
                    format!("execution provider envelope was invalid: {error}"),
                )),
            }
        }
    };
    let request = match ExecutionRequest::from_value(request_value) {
        Ok(request) => request,
        Err(error) => {
            return ProviderResponse {
                ok: false,
                receipt: None,
                failure: Some(error),
            }
        }
    };
    match provider.execute(&request) {
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
    }
}

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let provider = RustExecutionProvider::new();
    let mut line = Vec::new();
    loop {
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
        let response = match serde_json::from_slice::<Value>(&line) {
            Ok(input) => response_for(input, &provider),
            Err(error) => ProviderResponse {
                ok: false,
                receipt: None,
                failure: Some(failure(
                    "contract-failure",
                    format!("execution provider input was not valid JSON: {error}"),
                )),
            },
        };
        serde_json::to_writer(&mut stdout, &response)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}
