//! Multi-connection local HTTP fixture for Ollama transport tests. Every reply
//! closes its connection so each client request is observable separately.
#![allow(dead_code)]
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

pub enum Reply {
    Json(u16, String),
    Stream(Vec<Vec<u8>>, Duration),
    /// Drop the connection without any response bytes.
    Close,
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

pub fn serve<H>(addr: SocketAddr, handler: H) -> Fixture
where
    H: Fn(&Request, usize) -> Reply + Send + Sync + 'static,
{
    let listener = TcpListener::bind(addr).unwrap();
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
