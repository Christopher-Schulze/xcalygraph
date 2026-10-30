//! xcalygraph: dense, agent-readable code graphs and analyses for AI agents.

mod app;
mod cli;
mod domain;
mod extract;
mod infra;
mod resolve;

use std::io::{self, Write};
use std::process::ExitCode;

use cli::ExitStatus;

fn main() -> ExitCode {
    let invocation = match cli::parse(std::env::args_os().skip(1)) {
        Ok(invocation) => invocation,
        Err(error) => {
            write_stderr(&format!("xcalygraph: {error}; run 'xcalygraph help'\n"));
            return ExitStatus::Usage.into();
        }
    };
    let status = match cli::local_command(&invocation) {
        Some(command) => write_stdout(cli::render(command).as_bytes()),
        None => match app::run(&invocation) {
            Ok(output) => write_stdout(output.as_bytes()),
            Err(app::error::AppError::Ambiguous(candidates)) => {
                // Candidates are part of the answer: the agent picks one and repeats the call.
                let _ = write_stdout(candidates.as_bytes());
                ExitStatus::Ambiguous
            }
            Err(error) => {
                write_stderr(&format!("xcalygraph: {error}\n"));
                error.status()
            }
        },
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
