//! xcalygraph: dense, agent-readable code graphs and analyses for AI agents.

mod cli;

use std::io::{self, Write};
use std::process::ExitCode;

use cli::ExitStatus;

fn main() -> ExitCode {
    let status = match cli::parse(std::env::args_os().skip(1)) {
        Ok(command) => write_stdout(cli::render(command).as_bytes()),
        Err(error) => {
            write_stderr(&format!("xcalygraph: {error}; run 'xcalygraph help'\n"));
            ExitStatus::Usage
        }
    };
    status.into()
}

/// Writes command output; a reader that closed early (broken pipe) is not an error.
fn write_stdout(bytes: &[u8]) -> ExitStatus {
    let mut stdout = io::stdout().lock();
    match stdout.write_all(bytes).and_then(|()| stdout.flush()) {
        Ok(()) => ExitStatus::Answer,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitStatus::Answer,
        Err(error) => {
            write_stderr(&format!("xcalygraph: write stdout: {error}\n"));
            ExitStatus::Runtime
        }
    }
}

/// Best effort: a failing stderr leaves no channel to report on.
fn write_stderr(message: &str) {
    let _ = io::stderr().lock().write_all(message.as_bytes());
}
