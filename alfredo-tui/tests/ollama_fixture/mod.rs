//! Multi-connection local HTTP fixture for Ollama transport tests. Every reply
//! closes its connection so each client request is observable separately.
#![allow(dead_code)]
use std::{
    io::{ErrorKind, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub enum Reply {
    Json(u16, String),
    Stream(Vec<Vec<u8>>, Duration),
    /// Drop the connection without any response bytes.
    Close,
    /// Never answer; set the flag once the client closes the connection.
    Hold(Arc<AtomicBool>),
}

#[derive(Clone, Debug)]
pub struct Request {
    pub path: String,
    pub body: serde_json::Value,
}

pub struct Fixture {
    pub endpoint: String,
    pub requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

/// A free local port whose listener is closed: connections are refused until
/// `serve` binds it.
pub fn reserve() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

pub fn endpoint(addr: SocketAddr) -> String {
    format!("http://{addr}")
}

/// Bind `addr`, retrying briefly on `AddrInUse` (a short-lived squatter such as
/// a parallel test's ephemeral client port usually lets go within milliseconds).
fn bind_retrying(addr: SocketAddr, patience: Duration) -> std::io::Result<TcpListener> {
    let deadline = Instant::now() + patience;
    loop {
        match TcpListener::bind(addr) {
            Err(error) if error.kind() == ErrorKind::AddrInUse && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
            }
            other => return other,
        }
    }
}

/// Serve on the reserved `addr`. `reserve` closes its listener, so another test
/// or process may take the port before this binds (issue #300): after a short
/// retry this falls back to a fresh port, and `Fixture::endpoint` is always the
/// real address. Callers must therefore use `fixture.endpoint`, never a
/// precomputed `endpoint(addr)`; callers that must be refused on `addr` until
/// the server starts use `serve_at`.
pub fn serve<H>(addr: SocketAddr, handler: H) -> Fixture
where
    H: Fn(&Request, usize) -> Reply + Send + Sync + 'static,
{
    let listener = bind_retrying(addr, Duration::from_millis(100))
        .or_else(|_| TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    start(listener, handler)
}

/// Serve on exactly `addr`, for callers that handed `endpoint(addr)` to the code
/// under test while it was still refusing connections. A collision there is
/// genuine, so wait up to 3 s for the port and then panic naming it.
pub fn serve_at<H>(addr: SocketAddr, handler: H) -> Fixture
where
    H: Fn(&Request, usize) -> Reply + Send + Sync + 'static,
{
    let listener = bind_retrying(addr, Duration::from_secs(3))
        .unwrap_or_else(|error| panic!("fixture could not bind reserved {addr}: {error}"));
    start(listener, handler)
}

fn start<H>(listener: TcpListener, handler: H) -> Fixture
where
    H: Fn(&Request, usize) -> Reply + Send + Sync + 'static,
{
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let handler = Arc::new(handler);
    let handle = {
        let requests = requests.clone();
        let stop = stop.clone();
        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let requests = requests.clone();
                        let handler = handler.clone();
                        thread::spawn(move || handle(stream, &requests, &*handler));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(5)),
                }
            }
        })
    };
    Fixture {
        endpoint: endpoint(addr),
        requests,
        stop,
        handle: Some(handle),
    }
}

impl Fixture {
    pub fn paths(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.path.clone())
            .collect()
    }
    pub fn count(&self, path: &str) -> usize {
        self.paths().iter().filter(|seen| *seen == path).count()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn handle<H>(mut stream: TcpStream, requests: &Mutex<Vec<Request>>, handler: &H)
where
    H: Fn(&Request, usize) -> Reply + ?Sized,
{
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read_exact(&mut byte).is_err() {
            return;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).unwrap();
    let mut line = head.lines().next().unwrap().split(' ');
    let method = line.next().unwrap().to_string();
    let path = line.next().unwrap().to_string();
    let length: usize = head
        .lines()
        .find_map(|line| {
            line.to_lowercase()
                .strip_prefix("content-length: ")
                .map(str::to_owned)
        })
        .map(|value| value.parse().unwrap())
        .unwrap_or(0);
    let mut body = vec![0; length];
    if stream.read_exact(&mut body).is_err() {
        return;
    }
    let request = Request {
        path: format!("{method} {path}"),
        body: if body.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        },
    };
    let index = {
        let mut requests = requests.lock().unwrap();
        requests.push(request.clone());
        requests
            .iter()
            .filter(|seen| seen.path == request.path)
            .count()
            - 1
    };
    match handler(&request, index) {
        Reply::Close => {}
        Reply::Hold(closed) => {
            if matches!(stream.read(&mut byte), Ok(0)) {
                closed.store(true, Ordering::SeqCst);
            }
        }
        Reply::Json(status, body) => {
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
        Reply::Stream(chunks, pause) => {
            if stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n")
                .is_err()
            {
                return;
            }
            for chunk in chunks {
                if stream.write_all(&chunk).is_err() {
                    return;
                }
                let _ = stream.flush();
                thread::sleep(pause);
            }
        }
    }
}

pub fn done(text: &str) -> Reply {
    Reply::Stream(
        vec![format!(
            "{}\n",
            serde_json::json!({"message":{"content":text},"done":true})
        )
        .into_bytes()],
        Duration::ZERO,
    )
}

pub fn running(models: &[&str]) -> Reply {
    Reply::Json(
        200,
        serde_json::json!({"models": models.iter().map(|name| serde_json::json!({"name":name,"model":name})).collect::<Vec<_>>()})
            .to_string(),
    )
}

/// One streamed assistant reply asking for `calls` (name, arguments), then done.
pub fn tool_calls(calls: &[(&str, serde_json::Value)]) -> Reply {
    let calls: Vec<_> = calls
        .iter()
        .map(
            |(name, arguments)| serde_json::json!({"function":{"name":name,"arguments":arguments}}),
        )
        .collect();
    Reply::Stream(
        vec![
            format!(
                "{}\n",
                serde_json::json!({"message":{"role":"assistant","content":"","tool_calls":calls},"done":false})
            )
            .into_bytes(),
            format!("{}\n", serde_json::json!({"message":{"content":""},"done":true})).into_bytes(),
        ],
        Duration::ZERO,
    )
}

/// A scripted chat model: `POST /api/chat` number n gets `script[n]` (the last
/// entry repeats); every other request is answered as a server running `fixture`.
pub fn scripted_chat(script: Vec<Reply>) -> Fixture {
    serve(reserve(), move |request, index| {
        if request.path == "POST /api/chat" {
            script[index.min(script.len() - 1)].clone()
        } else {
            running(&["fixture"])
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(endpoint: &str) -> String {
        let mut stream = TcpStream::connect(endpoint.trim_start_matches("http://")).unwrap();
        stream
            .write_all(b"GET /api/tags HTTP/1.1\r\nHost: fixture\r\n\r\n")
            .unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        reply
    }

    #[test]
    fn serve_survives_another_listener_taking_the_reserved_port() {
        let squatter = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = squatter.local_addr().unwrap();
        let fixture = serve(addr, |_, _| done("ok"));
        assert_ne!(fixture.endpoint, endpoint(addr));
        assert!(get(&fixture.endpoint).contains("200 OK"));
        drop(squatter);
    }

    #[test]
    fn serve_binds_the_reserved_port_when_it_is_still_free() {
        let addr = reserve();
        let fixture = serve(addr, |_, _| done("ok"));
        assert_eq!(fixture.endpoint, endpoint(addr));
        assert!(get(&fixture.endpoint).contains("200 OK"));
    }

    #[test]
    fn serve_at_waits_for_a_port_that_is_released_shortly() {
        // The test owns the port from the start: binding port 0 hands us a free
        // one atomically. `reserve()` here would close the port and then rebind
        // it, and a parallel test could take it in between (issue #373).
        let squatter = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = squatter.local_addr().unwrap();
        let release = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            drop(squatter);
        });
        let fixture = serve_at(addr, |_, _| done("ok"));
        release.join().unwrap();
        assert_eq!(fixture.endpoint, endpoint(addr));
        assert!(get(&fixture.endpoint).contains("200 OK"));
    }
}
