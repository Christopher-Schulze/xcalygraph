//! Filesystem, index store and git access.

pub mod lock;
pub mod scan;
pub mod store;
#[cfg(test)]
pub mod testdir;
pub mod walk;
