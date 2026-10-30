//! Use-case errors and their exit statuses.

use std::fmt;
use std::io;

use crate::cli::ExitStatus;

#[derive(Debug)]
pub enum AppError {
    /// An IO operation failed; the string names what was attempted.
    Io(String, io::Error),
    /// `--read-only` found no index or a stale one.
    Stale(&'static str),
    /// A NAME matched several symbols; candidates are part of the answer.
    Ambiguous(String),
    /// Nothing matched.
    NoMatch(String),
    /// Invalid input detected after parsing (for example a root that is no directory).
    Invalid(String),
}

impl AppError {
    pub fn io(context: impl Into<String>) -> impl FnOnce(io::Error) -> Self {
        let context = context.into();
        move |error| Self::Io(context, error)
    }

    pub fn status(&self) -> ExitStatus {
        match self {
            Self::Io(..) | Self::Invalid(_) => ExitStatus::Runtime,
            Self::Stale(_) => ExitStatus::Stale,
            Self::Ambiguous(_) => ExitStatus::Ambiguous,
            Self::NoMatch(_) => ExitStatus::NoMatch,
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(context, error) => write!(formatter, "{context}: {error}"),
            Self::Stale(reason) => write!(formatter, "{reason} (--read-only)"),
            Self::Ambiguous(text) | Self::NoMatch(text) | Self::Invalid(text) => {
                formatter.write_str(text)
            }
        }
    }
}

impl std::error::Error for AppError {}
