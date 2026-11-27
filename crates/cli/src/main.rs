//! The `archviz` binary: see the archviz_cli library.

use std::io::{Read, Write};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use archviz_cli::Host;
use archviz_cli::host::run_client;
use archviz_core::Uuid;

/// The real clock and random ids (Q-49); core never sees either.
struct System;

impl Host for System {
    fn now_ms(&mut self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }

    fn new_uuid(&mut self) -> Uuid {
        Uuid::from_bytes(*uuid::Uuid::new_v4().as_bytes())
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut stdin = String::new();
    let _ = std::io::stdin().read_to_string(&mut stdin);
    let (out, status) = run_client(&args, &stdin, &mut System);
    let mut stdout = std::io::stdout();
    let _ = writeln!(stdout, "{out}");
    ExitCode::from(u8::try_from(status).unwrap_or(1))
}
