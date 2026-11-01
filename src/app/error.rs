//! Use-case errors and their exit statuses.

use std::fmt;
use std::io;

use crate::cli::ExitStatus;

#[derive(Debug)]
pub enum AppError {
    /// An IO operation failed; the string names what was attempted.
    Io(String, io::Error),
    /// An index-location operation failed, separately from source and other I/O.
    IndexIo(String, io::Error),
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

    pub fn index_io(context: impl Into<String>) -> impl FnOnce(io::Error) -> Self {
        let context = context.into();
        move |error| Self::IndexIo(context, error)
    }

    /// Only actual location failures can select a fallback, never lock contention or source I/O.
    pub fn is_index_location_failure(&self) -> bool {
        matches!(self, Self::IndexIo(_, error) if matches!(error.kind(),
            io::ErrorKind::PermissionDenied
                | io::ErrorKind::ReadOnlyFilesystem
                | io::ErrorKind::StorageFull
                | io::ErrorKind::QuotaExceeded
                | io::ErrorKind::AlreadyExists
                | io::ErrorKind::NotFound
                | io::ErrorKind::NotADirectory
                | io::ErrorKind::IsADirectory
        ))
    }

    pub fn status(&self) -> ExitStatus {
        match self {
            Self::Io(..) | Self::IndexIo(..) | Self::Invalid(_) => ExitStatus::Runtime,
            Self::Stale(_) => ExitStatus::Stale,
            Self::Ambiguous(_) => ExitStatus::Ambiguous,
            Self::NoMatch(_) => ExitStatus::NoMatch,
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(context, error) | Self::IndexIo(context, error) => {
                write!(formatter, "{context}: {error}")
            }
            Self::Stale(reason) => write!(formatter, "{reason} (--read-only)"),
            Self::Ambiguous(text) | Self::NoMatch(text) | Self::Invalid(text) => {
                formatter.write_str(text)
            }
        }
    }
}

impl std::error::Error for AppError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_distinguishes_index_location_from_source_and_lock_errors() {
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::ReadOnlyFilesystem,
            io::ErrorKind::StorageFull,
            io::ErrorKind::QuotaExceeded,
            io::ErrorKind::AlreadyExists,
            io::ErrorKind::NotFound,
            io::ErrorKind::NotADirectory,
            io::ErrorKind::IsADirectory,
        ] {
            assert!(AppError::IndexIo("index".into(), kind.into()).is_index_location_failure());
            assert!(!AppError::Io("source".into(), kind.into()).is_index_location_failure());
        }
        for kind in [
            io::ErrorKind::TimedOut,
            io::ErrorKind::WouldBlock,
            io::ErrorKind::Interrupted,
        ] {
            assert!(!AppError::IndexIo("lock".into(), kind.into()).is_index_location_failure());
        }
    }
}
