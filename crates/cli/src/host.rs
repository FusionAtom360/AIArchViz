//! The host process (spec 3.1, DEC-13). A host holds a project open and
//! serves the command API over a local named pipe; any other client attaches
//! to it instead of opening the project files.
//!
//! While it serves, the host keeps `host.json` next to project.lock with the
//! pipe name and a random 256-bit session token, and removes it when it
//! stops. A request is one JSON line `{"token", "args", "stdin"}` and the
//! answer one JSON line `{"stdout", "status"}`, where `stdout` and `status`
//! are exactly what `archviz` would print and exit with. Requests are run one
//! at a time under the project's mutex, so clients never interleave a commit.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, prelude::*};
use serde::{Deserialize, Serialize};

use crate::{CliError, ErrorCode, Host, respond, run, run_on, store};

/// The file a running host keeps in the project folder.
pub const HOST_FILE: &str = "host.json";

/// How long a client keeps trying to reach a host that holds the lock but
/// has not accepted its connection yet.
const ATTACH_WAIT: Duration = Duration::from_secs(10);

/// What a host holds: the open project and its clock and id source.
pub struct Shared {
    pub project: store::Project,
    pub host: Box<dyn Host + Send>,
}

/// Called after a request that committed, outside the project's mutex.
pub type Notify = Box<dyn Fn() + Send + Sync>;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostFile {
    pipe: String,
    token: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRequest {
    token: String,
    args: Vec<String>,
    stdin: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireResponse {
    stdout: String,
    status: i32,
}

fn io(e: std::io::Error) -> CliError {
    CliError::new(ErrorCode::Io, e.to_string())
}

/// `n` random bytes from the OS, in lowercase hex.
fn random_hex(n: usize) -> Result<String, CliError> {
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).map_err(|e| CliError::new(ErrorCode::Io, e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Compares two tokens without stopping at the first difference.
fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn connect(pipe: &str) -> std::io::Result<Stream> {
    Stream::connect(pipe.to_ns_name::<GenericNamespaced>()?)
}

/// A running host. Dropping it stops serving and removes host.json; the
/// project stays open for as long as its `Shared` lives.
pub struct Server {
    dir: PathBuf,
    pipe: String,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag. The connection stays
        // open until the loop has ended: closed at once, it would be lost if
        // the loop had not yet started waiting (the server's connect then
        // fails with ERROR_NO_DATA, which interprocess answers by waiting
        // for another client), and `join` would never return (Q-80).
        let wake = connect(&self.pipe);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        drop(wake);
        let _ = std::fs::remove_file(self.dir.join(HOST_FILE));
    }
}

/// Starts serving `shared`'s project, whose folder is `dir`: creates the
/// pipe, then writes host.json. Connection handlers hold the project only
/// weakly, so dropping the last `Arc` closes it and releases its lock.
pub fn serve(shared: &Arc<Mutex<Shared>>, dir: &Path, notify: Notify) -> Result<Server, CliError> {
    let pipe = format!("archviz-{}", random_hex(16)?);
    let token: Arc<str> = Arc::from(random_hex(32)?);
    let listener = ListenerOptions::new()
        .name(
            pipe.as_str()
                .to_ns_name::<GenericNamespaced>()
                .map_err(io)?,
        )
        .create_sync()
        .map_err(io)?;
    let file = serde_json::to_string(&HostFile {
        pipe: pipe.clone(),
        token: token.to_string(),
    })
    .map_err(|e| CliError::new(ErrorCode::Io, e.to_string()))?;
    // Written whole, then renamed, so a client never reads half a file.
    let partial = dir.join("host.json.partial");
    std::fs::write(&partial, file).map_err(io)?;
    std::fs::rename(&partial, dir.join(HOST_FILE)).map_err(io)?;

    let stop = Arc::new(AtomicBool::new(false));
    let weak = Arc::downgrade(shared);
    let notify: Arc<Notify> = Arc::new(notify);
    let flag = stop.clone();
    let accept = std::thread::spawn(move || {
        for conn in listener.incoming() {
            if flag.load(Ordering::SeqCst) {
                break;
            }
            let Ok(conn) = conn else { continue };
            let (weak, token, notify) = (weak.clone(), token.clone(), notify.clone());
            std::thread::spawn(move || {
                let _ = handle(conn, &weak, &token, &notify);
            });
        }
    });
    Ok(Server {
        dir: dir.to_path_buf(),
        pipe,
        stop,
        accept: Some(accept),
    })
}

/// Answers one connection: one request line, one response line.
fn handle(
    conn: Stream,
    shared: &Weak<Mutex<Shared>>,
    token: &str,
    notify: &Notify,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(conn);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let (stdout, status) = match serde_json::from_str::<WireRequest>(&line) {
        Err(e) => respond(Err(CliError::new(
            ErrorCode::BadRequest,
            format!("invalid host request: {e}"),
        ))),
        Ok(req) if !same_token(&req.token, token) => respond(Err(CliError::new(
            ErrorCode::Unauthorized,
            "wrong session token".to_string(),
        ))),
        Ok(req) => match shared.upgrade() {
            None => respond(Err(CliError::new(
                ErrorCode::Io,
                "the host closed this project".to_string(),
            ))),
            Some(shared) => {
                let mut guard = shared.lock().unwrap_or_else(PoisonError::into_inner);
                let state = &mut *guard;
                let before = state.project.history.version();
                let out = run_on(&mut state.project, &req.args, &req.stdin, &mut *state.host);
                let changed = state.project.history.version() != before;
                drop(guard);
                if changed {
                    notify();
                }
                out
            }
        },
    };
    let mut reply =
        serde_json::to_string(&WireResponse { stdout, status }).map_err(std::io::Error::other)?;
    reply.push('\n');
    reader.get_mut().write_all(reply.as_bytes())?;
    reader.get_mut().flush()
}

/// Sends one request to the host on `pipe`; returns its stdout and status.
pub fn request(
    pipe: &str,
    token: &str,
    args: &[String],
    stdin: &str,
) -> std::io::Result<(String, i32)> {
    exchange(connect(pipe)?, token, args, stdin)
}

fn exchange(
    stream: Stream,
    token: &str,
    args: &[String],
    stdin: &str,
) -> std::io::Result<(String, i32)> {
    let mut line = serde_json::to_string(&WireRequest {
        token: token.to_string(),
        args: args.to_vec(),
        stdin: stdin.to_string(),
    })
    .map_err(std::io::Error::other)?;
    line.push('\n');
    let mut reader = BufReader::new(stream);
    reader.get_mut().write_all(line.as_bytes())?;
    reader.get_mut().flush()?;
    let mut answer = String::new();
    reader.read_line(&mut answer)?;
    let response: WireResponse = serde_json::from_str(&answer).map_err(std::io::Error::other)?;
    Ok((response.stdout, response.status))
}

fn read_host_file(dir: &Path) -> Option<HostFile> {
    let text = std::fs::read_to_string(dir.join(HOST_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// What `archviz` runs (3.1): if a host serves the project, the request goes
/// to it; otherwise the project files are opened directly under the lock.
/// A host.json whose pipe does not answer while the lock is free was left by
/// a host that is gone, so it is removed and the request runs directly. A
/// request whose connection breaks after it was sent is reported as IO, never
/// sent twice.
pub fn run_client(args: &[String], stdin: &str, host: &mut dyn Host) -> (String, i32) {
    let [sub, dir] = args else {
        return run(args, stdin, host);
    };
    if sub == "new" {
        return run(args, stdin, host);
    }
    let dir = Path::new(dir);
    let deadline = Instant::now() + ATTACH_WAIT;
    loop {
        let Some(file) = read_host_file(dir) else {
            return run(args, stdin, host);
        };
        match connect(&file.pipe) {
            Ok(stream) => {
                return exchange(stream, &file.token, args, stdin)
                    .unwrap_or_else(|e| respond(Err(io(e))));
            }
            Err(_) => match store::open(dir) {
                Ok(mut project) => {
                    let _ = std::fs::remove_file(dir.join(HOST_FILE));
                    return run_on(&mut project, args, stdin, host);
                }
                Err(e) if e.code == ErrorCode::Locked && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return respond(Err(e)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{random_hex, same_token};

    #[test]
    fn tokens_compare_exactly() {
        assert!(same_token("abcd", "abcd"));
        assert!(!same_token("abcd", "abce"));
        assert!(!same_token("abcd", "abc"));
        assert!(!same_token("", "a"));
    }

    #[test]
    fn random_hex_has_two_digits_per_byte_and_differs() -> Result<(), String> {
        let a = random_hex(32).map_err(|e| e.message)?;
        let b = random_hex(32).map_err(|e| e.message)?;
        assert_eq!(a.len(), 64);
        assert!(
            a.bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_ne!(a, b);
        Ok(())
    }
}
