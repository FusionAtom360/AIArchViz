//! The project on disk (Q-47): a folder holding genesis.json (the canonical
//! genesis document), log.jsonl (one canonical log entry per line, append
//! only) and project.lock. Opening a project takes an exclusive OS lock on
//! project.lock for as long as the Project lives, then replays the log.
//!
//! A commit is answered only after its whole line, newline included, is
//! flushed. A final segment with no newline is therefore an append that a
//! crash cut short: it was never committed, so `open` ignores it and the
//! next `append` truncates it away (Q-73).

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use archviz_core::{
    Document, History, LogEntry, from_canonical_json, log_from_jsonl, to_canonical_json,
};

use crate::{CliError, ErrorCode};

const GENESIS: &str = "genesis.json";
const LOG: &str = "log.jsonl";
const LOCK: &str = "project.lock";

/// An open project: the lock is held until it is dropped.
pub struct Project {
    dir: PathBuf,
    _lock: File,
    /// Length in bytes of the committed log, up to its last newline.
    log_len: u64,
    pub history: History,
}

/// The committed part of a log: everything up to and including its last
/// newline. Whatever follows is a torn append.
fn committed(log: &[u8]) -> &[u8] {
    match log.iter().rposition(|&b| b == b'\n') {
        Some(i) => log.get(..=i).unwrap_or(log),
        None => &[],
    }
}

fn io(e: std::io::Error) -> CliError {
    CliError::new(ErrorCode::Io, e.to_string())
}

/// Takes the exclusive lock on the project folder.
fn lock(dir: &Path) -> Result<File, CliError> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(LOCK))
        .map_err(io)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(CliError::new(
            ErrorCode::Locked,
            "another process has this project open".to_string(),
        )),
        Err(TryLockError::Error(e)) => Err(io(e)),
    }
}

/// Creates a new project folder with `genesis` and an empty log.
pub fn create(dir: &Path, genesis: &Document) -> Result<(), CliError> {
    if dir.join(GENESIS).exists() {
        return Err(CliError::new(
            ErrorCode::ProjectExists,
            format!("{} already holds a project", dir.display()),
        ));
    }
    std::fs::create_dir_all(dir).map_err(io)?;
    let _lock = lock(dir)?;
    let text = to_canonical_json(genesis)
        .map_err(|e| CliError::new(ErrorCode::CommandFailed, e.to_string()))?;
    std::fs::write(dir.join(GENESIS), text).map_err(io)?;
    std::fs::write(dir.join(LOG), "").map_err(io)?;
    Ok(())
}

/// Opens a project: locks it, reads the genesis document and replays the log.
pub fn open(dir: &Path) -> Result<Project, CliError> {
    if !dir.join(GENESIS).is_file() {
        return Err(CliError::new(
            ErrorCode::NotAProject,
            format!("{} is not a project folder", dir.display()),
        ));
    }
    let lock = lock(dir)?;
    let corrupt =
        |what: &str, e: String| CliError::new(ErrorCode::CorruptProject, format!("{what}: {e}"));
    let genesis = std::fs::read_to_string(dir.join(GENESIS)).map_err(io)?;
    let genesis = from_canonical_json(&genesis).map_err(|e| corrupt(GENESIS, e.to_string()))?;
    let log = std::fs::read(dir.join(LOG)).map_err(io)?;
    let log = committed(&log);
    let log_len = u64::try_from(log.len()).map_err(|e| corrupt(LOG, e.to_string()))?;
    let log = std::str::from_utf8(log).map_err(|e| corrupt(LOG, e.to_string()))?;
    let entries = log_from_jsonl(log).map_err(|e| corrupt(LOG, e.to_string()))?;
    let mut history = History::new(genesis);
    history
        .replay(&entries)
        .map_err(|e| corrupt(LOG, e.to_string()))?;
    Ok(Project {
        dir: dir.to_path_buf(),
        _lock: lock,
        log_len,
        history,
    })
}

impl Project {
    /// Appends one committed entry to the log and flushes it to disk, first
    /// truncating any torn append after the committed part.
    pub fn append(&mut self, entry: &LogEntry) -> Result<(), CliError> {
        let mut line = to_canonical_json(entry)
            .map_err(|e| CliError::new(ErrorCode::CommandFailed, e.to_string()))?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .write(true)
            .open(self.dir.join(LOG))
            .map_err(io)?;
        file.set_len(self.log_len).map_err(io)?;
        file.seek(SeekFrom::Start(self.log_len)).map_err(io)?;
        file.write_all(line.as_bytes()).map_err(io)?;
        file.sync_data().map_err(io)?;
        let added = u64::try_from(line.len()).map_err(|e| io(std::io::Error::other(e)))?;
        self.log_len = self
            .log_len
            .checked_add(added)
            .ok_or_else(|| io(std::io::Error::other("the log is too long")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::committed;

    #[test]
    fn the_committed_part_ends_at_the_last_newline() {
        let table: [(&[u8], &[u8]); 6] = [
            (b"", b""),
            (b"\n", b"\n"),
            (b"{\"a\":1}\n", b"{\"a\":1}\n"),
            (b"{\"a\":1}\n{\"a\"", b"{\"a\":1}\n"),
            (b"{\"a\"", b""),
            (b"{\"a\":1}\n\xC3", b"{\"a\":1}\n"),
        ];
        for (log, expected) in table {
            assert_eq!(committed(log), expected, "{log:?}");
        }
    }
}
