// Issue #744: Enter on a chat draft, through the real binary on a pseudo-terminal and a
// fake Ollama server. A path-first prompt reaches the model as chat text; a mistyped
// command is refused in the footer and never sends a request.
use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::fd::FromRawFd;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

type Requests = Arc<Mutex<Vec<(String, String)>>>;

fn serve() -> (String, Requests) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let requests: Requests = Arc::default();
    let seen = requests.clone();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let seen = seen.clone();
            thread::spawn(move || answer(stream, &seen));
        }
    });
    (endpoint, requests)
}

fn answer(mut stream: std::net::TcpStream, seen: &Requests) {
    let mut raw = Vec::new();
    let mut byte = [0u8; 1];
    while !raw.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            return;
        }
        raw.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&raw).to_string();
    let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
    let length = head
        .lines()
        .find_map(|line| {
            line.to_lowercase()
                .strip_prefix("content-length:")
                .map(str::to_owned)
        })
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    let _ = stream.read_exact(&mut body);
    let body = String::from_utf8_lossy(&body).to_string();
    let payload = if head.starts_with("GET") {
        r#"{"models":[{"name":"fixture"}]}"#.to_string()
    } else {
        seen.lock().unwrap().push((path.clone(), body));
        if path == "/api/chat" {
            "{\"message\":{\"content\":\"PATH_REPLY_MARKER\"},\"done\":false}\n\
             {\"message\":{\"content\":\"\"},\"done\":true}\n"
                .to_string()
        } else {
            "{\"done\":true,\"done_reason\":\"load\"}\n".to_string()
        }
    };
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n{payload}"
    );
}

struct Terminal {
    child: Child,
    input: File,
    output: Arc<Mutex<Vec<u8>>>,
    dir: std::path::PathBuf,
}

impl Terminal {
    fn start(endpoint: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pfe{}", std::process::id()));
        let state = dir.join("s");
        let repo = dir.join("r");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q", "--template="])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        let (mut master, mut slave) = (0, 0);
        let size = libc::winsize {
            ws_row: 24,
            ws_col: 100,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: openpty fills two descriptors we then own; `size` outlives the call.
        let opened = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                &size,
            )
        };
        assert_eq!(opened, 0, "openpty failed");
        // SAFETY: both descriptors are valid and owned only by these Files.
        let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
        let child = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
            .args(["--model", "fixture", "--endpoint", endpoint])
            .env("TERM", "xterm-256color")
            .env("ALFREDO_STATE_DIR", &state)
            .current_dir(&repo)
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave))
            .spawn()
            .unwrap();
        let output: Arc<Mutex<Vec<u8>>> = Arc::default();
        let (mut reader, sink) = (master.try_clone().unwrap(), output.clone());
        let mut reply = master.try_clone().unwrap();
        thread::spawn(move || {
            let mut chunk = [0u8; 65536];
            while let Ok(count @ 1..) = reader.read(&mut chunk) {
                sink.lock().unwrap().extend_from_slice(&chunk[..count]);
                if chunk[..count].windows(4).any(|w| w == b"\x1b[6n") {
                    let _ = reply.write_all(b"\x1b[1;1R");
                }
            }
        });
        Self {
            child,
            input: master,
            output,
            dir,
        }
    }

    fn seen(&self, text: &str) -> bool {
        String::from_utf8_lossy(&self.output.lock().unwrap()).contains(text)
    }

    fn count(&self, text: &str) -> usize {
        String::from_utf8_lossy(&self.output.lock().unwrap())
            .matches(text)
            .count()
    }

    fn wait_for(&self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.seen(text) {
            assert!(
                Instant::now() < deadline,
                "never saw {text:?}: {:?}",
                String::from_utf8_lossy(&self.output.lock().unwrap())
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn type_line(&mut self, text: &str) {
        self.input.write_all(text.as_bytes()).unwrap();
        self.input.write_all(b"\r").unwrap();
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn chats(requests: &Requests) -> Vec<String> {
    let found = requests.lock().unwrap();
    found
        .iter()
        .filter(|(path, _)| path == "/api/chat")
        .map(|(_, body)| body.clone())
        .collect()
}

#[test]
fn enter_sends_a_path_first_prompt_as_chat_and_refuses_a_mistyped_command() {
    let (endpoint, requests) = serve();
    let mut terminal = Terminal::start(&endpoint);
    terminal.wait_for("ALFREDO");
    thread::sleep(Duration::from_secs(1));
    terminal.type_line("/gp fix it");
    // The screen is diffed, so the footer arrives in pieces; its first words are intact.
    terminal.wait_for("Unknown command");
    thread::sleep(Duration::from_millis(500));
    assert!(chats(&requests).is_empty(), "a mistyped command was sent");
    terminal.type_line("\u{15}/etc/hosts what is this");
    let deadline = Instant::now() + Duration::from_secs(30);
    while chats(&requests).is_empty() {
        assert!(
            Instant::now() < deadline,
            "the path-first prompt never reached the model"
        );
        thread::sleep(Duration::from_millis(50));
    }
    let sent = chats(&requests);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].contains("/etc/hosts what is this"), "{}", sent[0]);
    assert_eq!(terminal.count("Unknown command"), 1, "only /gp is refused");
}
