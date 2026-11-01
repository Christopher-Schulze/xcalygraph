//! Exclusive writer lock on `<index>/lock` (`docs/spec.md` section 4, Locking).

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

pub const WAIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(20);

/// Held while a writer refreshes the index; released on drop.
#[derive(Debug)]
pub struct WriterLock {
    file: File,
}

impl WriterLock {
    /// Waits up to `wait` for the lock; times out with `ErrorKind::TimedOut`.
    pub fn acquire(index_dir: &Path, wait: Duration) -> io::Result<Self> {
        let path = index_dir.join("lock");
        let file = match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                super::file_access::open_existing_regular(&path, true)?
            }
            Err(error) => return Err(error),
        };
        let deadline = Instant::now() + wait;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { file }),
                Err(TryLockError::Error(error)) => return Err(error),
                Err(TryLockError::WouldBlock) if Instant::now() >= deadline => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("index locked by another writer for {}s", wait.as_secs()),
                    ));
                }
                Err(TryLockError::WouldBlock) => thread::sleep(POLL),
            }
        }
    }
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::testdir::TestDir;

    #[test]
    fn second_writer_waits_then_times_out_and_succeeds_after_release() -> io::Result<()> {
        let dir = TestDir::new("lock")?;
        let first = WriterLock::acquire(dir.path(), WAIT)?;
        let started = Instant::now();
        let second = WriterLock::acquire(dir.path(), Duration::from_millis(100));
        assert_eq!(
            second.map(|_| ()).map_err(|e| e.kind()),
            Err(io::ErrorKind::TimedOut)
        );
        assert!(started.elapsed() >= Duration::from_millis(100));
        drop(first);
        let third = WriterLock::acquire(dir.path(), Duration::from_millis(100));
        assert!(third.is_ok());
        Ok(())
    }

    #[test]
    fn waiting_writer_gets_the_lock_when_released() -> io::Result<()> {
        let dir = TestDir::new("lock-wait")?;
        let first = WriterLock::acquire(dir.path(), WAIT)?;
        let path = dir.path().to_path_buf();
        let waiter =
            thread::spawn(move || WriterLock::acquire(&path, Duration::from_secs(5)).map(|_| ()));
        thread::sleep(Duration::from_millis(100));
        drop(first);
        let joined = waiter
            .join()
            .map_err(|_| io::Error::other("waiter panicked"))?;
        assert!(joined.is_ok());
        Ok(())
    }
}
