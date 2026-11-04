//! Filesystem, index store and git access.

pub mod alloc_stats;
pub mod file_access;
pub mod git;
pub mod lock;
pub mod scan;
pub mod store;
#[cfg(test)]
pub mod testdir;
pub mod walk;
