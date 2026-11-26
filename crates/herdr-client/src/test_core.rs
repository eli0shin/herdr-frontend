//! A scripted fake core behind a real Unix socket, for supervisor and watcher tests: they reach it
//! through `Transport::Socket` like any core.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use herdr_proto::{Decoder, Frame, Kind};
use serde_json::{Value, json};

use crate::Transport;
use crate::connection::USES;

static NEXT: AtomicU32 = AtomicU32::new(0);

/// A listening socket; each accepted connection runs `script` on its own thread.
pub(crate) struct FakeCore {
    path: PathBuf,
}

impl FakeCore {
    pub fn serve(script: impl Fn(Peer) + Send + Sync + 'static) -> FakeCore {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(format!("/tmp/hfc-fake-{}-{id}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let script = std::sync::Arc::new(script);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let script = script.clone();
                std::thread::spawn(move || {
                    script(Peer {
                        stream,
                        decoder: Decoder::new(),
                    })
                });
            }
        });
        FakeCore { path }
    }

    pub fn transport(&self) -> Transport {
        Transport::Socket(self.path.clone())
    }
}

impl Drop for FakeCore {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The core's end of one connection.
pub(crate) struct Peer {
    stream: UnixStream,
    decoder: Decoder,
}

impl Peer {
    /// Reads the plain-mode hello line and answers it as a compatible 0.1.22 core.
    pub fn answer_hello(&mut self) -> Option<()> {
        let mut byte = [0u8];
        loop {
            self.stream.read_exact(&mut byte).ok()?;
            if byte[0] == b'\n' {
                break;
            }
        }
        let reply = json!({"jsonrpc": "2.0", "id": 0, "result": {
            "generation": 1, "version": "0.1.22", "core": {"id": "c1", "name": "fake"},
            "framed": true, "methods": USES, "features": ["framed", "attach", "events"],
        }});
        let mut line = serde_json::to_vec(&reply).unwrap();
        line.push(b'\n');
        self.stream.write_all(&line).ok()
    }

    /// The next JSON-RPC message the client sent; `None` once it hangs up.
    pub fn message(&mut self) -> Option<Value> {
        let mut buf = [0u8; 4096];
        loop {
            match self.decoder.next_frame() {
                Ok(Some(frame)) if frame.channel == 0 && frame.kind == Kind::Data => {
                    return serde_json::from_slice(&frame.payload).ok();
                }
                Ok(Some(_)) => continue,
                Ok(None) => {}
                Err(_) => return None,
            }
            let n = self.stream.read(&mut buf).ok()?;
            if n == 0 {
                return None;
            }
            self.decoder.push(&buf[..n]);
        }
    }

    pub fn reply(&mut self, id: &Value, result: Value) -> Option<()> {
        let message = json!({"jsonrpc": "2.0", "id": id, "result": result});
        let frame = Frame::data(0, serde_json::to_vec(&message).unwrap());
        self.stream.write_all(&frame.encode().unwrap()).ok()
    }

    /// Never answers, not even hello: reads raw bytes until the client hangs up.
    pub fn hold(mut self) {
        let mut buf = [0u8; 4096];
        while self.stream.read(&mut buf).is_ok_and(|n| n > 0) {}
    }

    /// Reads until the client hangs up, answering nothing.
    pub fn ignore(mut self) {
        while self.message().is_some() {}
    }
}
