//! A protocol-level client: spawns the real `ginlsp-next` binary and talks Content-Length
//! framed JSON-RPC to it over pipes.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(60);

pub struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    incoming: Receiver<Value>,
    /// Messages read while waiting for something else.
    backlog: VecDeque<Value>,
    next_id: i64,
}

fn read_framed(reader: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0u8; length?];
    std::io::Read::read_exact(reader, &mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

pub fn frame(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

impl Client {
    pub fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ginlsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the server starts");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped");
        let (send, incoming) = channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Some(message) = read_framed(&mut reader) {
                if send.send(message).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            incoming,
            backlog: VecDeque::new(),
            next_id: 1,
        }
    }

    /// Start and complete the `initialize` handshake with these workspace roots.
    pub fn initialized(roots: &[&std::path::Path]) -> Self {
        let mut client = Self::start();
        let folders: Vec<Value> = roots
            .iter()
            .map(|r| json!({"uri": uri(r), "name": "w"}))
            .collect();
        let reply = client.request("initialize", json!({"processId": null, "rootUri": null, "capabilities": {}, "workspaceFolders": folders}));
        assert!(reply.get("result").is_some(), "{reply}");
        client.notify("initialized", json!({}));
        client
    }

    pub fn send_raw(&mut self, bytes: &[u8]) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        stdin.write_all(bytes).unwrap();
        stdin.flush().unwrap();
    }

    pub fn send(&mut self, message: &Value) {
        self.send_raw(&frame(&message.to_string()));
    }

    pub fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Send a request and return the response with the same id.
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        self.wait(|m| m.get("id") == Some(&json!(id)) && m.get("method").is_none())
    }

    /// The next message (backlog first) satisfying `matches`; others are kept.
    pub fn wait(&mut self, matches: impl Fn(&Value) -> bool) -> Value {
        if let Some(i) = self.backlog.iter().position(&matches) {
            return self.backlog.remove(i).unwrap();
        }
        loop {
            match self.incoming.recv_timeout(WAIT) {
                Ok(message) if matches(&message) => return message,
                Ok(message) => self.backlog.push_back(message),
                Err(RecvTimeoutError::Timeout) => panic!("timed out; backlog: {:?}", self.backlog),
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("the server closed its output; backlog: {:?}", self.backlog)
                }
            }
        }
    }

    /// The next `publishDiagnostics` for `uri`.
    pub fn diagnostics(&mut self, uri: &str) -> Vec<Value> {
        let message = self.wait(|m| {
            m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri
        });
        message["params"]["diagnostics"].as_array().unwrap().clone()
    }

    /// Whether nothing satisfying `matches` arrives within `ms` milliseconds.
    pub fn quiet(&mut self, ms: u64, matches: impl Fn(&Value) -> bool) -> bool {
        if self.backlog.iter().any(&matches) {
            return false;
        }
        let deadline = std::time::Instant::now() + Duration::from_millis(ms);
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            match self.incoming.recv_timeout(left) {
                Ok(message) if matches(&message) => return false,
                Ok(message) => self.backlog.push_back(message),
                Err(_) => break,
            }
        }
        true
    }

    pub fn open(&mut self, uri: &str, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": uri, "languageId": "gin", "version": 1, "text": text}}),
        );
    }

    pub fn change(&mut self, uri: &str, version: i32, text: &str) {
        self.notify("textDocument/didChange", json!({"textDocument": {"uri": uri, "version": version}, "contentChanges": [{"text": text}]}));
    }

    pub fn hover(&mut self, uri: &str, line: u32, character: u32) -> Value {
        self.request("textDocument/hover", json!({"textDocument": {"uri": uri}, "position": {"line": line, "character": character}}))
    }

    /// `shutdown` then `exit`; the process exit code.
    pub fn shutdown(mut self) -> i32 {
        let reply = self.request("shutdown", Value::Null);
        assert_eq!(reply["result"], Value::Null, "{reply}");
        self.notify("exit", Value::Null);
        self.finish()
    }

    pub fn finish(mut self) -> i32 {
        drop(self.stdin.take());
        self.child.wait().unwrap().code().unwrap_or(-1)
    }
}

pub fn uri(path: &std::path::Path) -> String {
    lsp_types::Url::from_file_path(path).unwrap().to_string()
}
