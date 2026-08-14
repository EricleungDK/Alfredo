use albert_mission_control::execution::{
    ExecutionReceipt, ExecutionRequest, RustExecutionProvider, StructuredFailure,
};
use serde::Serialize;
use serde_json::Value;
use std::io::{self, BufRead, Write};

#[derive(Serialize)]
struct ProviderResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt: Option<ExecutionReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<StructuredFailure>,
}

fn failure(code: &str, message: String) -> StructuredFailure {
    StructuredFailure {
        code: code.to_owned(),
        message,
        recoverable: true,
    }
}

fn response_for(input: Value, provider: &RustExecutionProvider) -> ProviderResponse {
    let request_value = match input {
        Value::Object(mut object) => object.remove("request").unwrap_or(Value::Null),
        _ => Value::Null,
    };
    if request_value.is_null() {
        return ProviderResponse {
            ok: false,
            receipt: None,
            failure: Some(failure(
                "contract-failure",
                "execution provider input must contain a request object".to_owned(),
            )),
        };
    }
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
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let provider = RustExecutionProvider::new();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
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
