//! The knowledge-base operation lock.
//!
//! Only one operation at a time may change the knowledge base's files or git
//! state: a write, a capture, a sync, or a long-running job that works on the
//! folder for minutes. Whoever needs the knowledge base asks for the lock and
//! says what it is doing; everyone else either waits a bounded time or gets a
//! [`Busy`] answer that names the operation in the way. Nothing ever blocks
//! forever.
//!
//! The guard is `Send`, so a long-running job can take the lock on one thread
//! and release it on another. Dropping the guard (also during a panic)
//! releases the lock.

use std::fmt;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// A cloneable handle to the one lock of the app (clones share the lock).
#[derive(Clone, Default)]
pub struct KbLock(Arc<Inner>);

#[derive(Default)]
struct Inner {
    holder: Mutex<Option<Holder>>,
    released: Condvar,
}

struct Holder {
    operation: String,
    since: Instant,
}

/// Holds the lock until dropped.
#[must_use = "the lock is released as soon as the guard is dropped"]
pub struct KbLockGuard {
    lock: KbLock,
}

/// The lock is held by another operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Busy {
    /// What the holder said it is doing (user-facing, e.g. `同步`).
    pub operation: String,
    /// How long it has held the lock so far.
    pub held_for: Duration,
}

impl fmt::Display for Busy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "知识库正忙：正在{}（已 {} 秒），请稍后再试",
            self.operation,
            self.held_for.as_secs()
        )
    }
}

impl std::error::Error for Busy {}

impl KbLock {
    /// Takes the lock if it is free right now. Meant for long-running jobs
    /// that would rather not start than wait.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn try_acquire(&self, operation: &str) -> Result<KbLockGuard, Busy> {
        self.acquire(operation, Duration::ZERO)
    }

    /// Takes the lock, waiting up to `wait` for the current holder to finish.
    pub fn acquire(&self, operation: &str, wait: Duration) -> Result<KbLockGuard, Busy> {
        let deadline = Instant::now() + wait;
        let mut holder = self.holder();
        loop {
            let Some(current) = holder.as_ref() else {
                *holder = Some(Holder {
                    operation: operation.to_string(),
                    since: Instant::now(),
                });
                return Ok(KbLockGuard { lock: self.clone() });
            };
            let now = Instant::now();
            if now >= deadline {
                return Err(Busy {
                    operation: current.operation.clone(),
                    held_for: now.duration_since(current.since),
                });
            }
            holder = self
                .0
                .released
                .wait_timeout(holder, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Who holds the lock at the moment, if anyone.
    pub fn held_by(&self) -> Option<Busy> {
        self.holder().as_ref().map(|current| Busy {
            operation: current.operation.clone(),
            held_for: current.since.elapsed(),
        })
    }

    // The inner mutex only guards a few assignments, so a poisoned mutex still
    // holds consistent data.
    fn holder(&self) -> MutexGuard<'_, Option<Holder>> {
        self.0.holder.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for KbLockGuard {
    fn drop(&mut self) {
        *self.lock.holder() = None;
        self.lock.0.released.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn one_holder_at_a_time_and_busy_names_the_operation() {
        let lock = KbLock::default();
        assert_eq!(lock.held_by(), None);

        let guard = lock.try_acquire("同步").unwrap();
        let busy = lock.try_acquire("写入").err().unwrap();
        assert_eq!(busy.operation, "同步");
        assert!(busy.to_string().contains("正在同步"), "{busy}");
        assert_eq!(lock.held_by().unwrap().operation, "同步");

        drop(guard);
        assert_eq!(lock.held_by(), None);
        let _again = lock.try_acquire("写入").unwrap();
    }

    #[test]
    fn clones_share_the_lock() {
        let lock = KbLock::default();
        let other = lock.clone();
        let _guard = lock.try_acquire("入库").unwrap();
        assert_eq!(other.try_acquire("写入").err().unwrap().operation, "入库");
    }

    #[test]
    fn acquire_waits_for_the_holder_to_finish() {
        let lock = KbLock::default();
        let guard = lock.try_acquire("同步").unwrap();
        let releaser = thread::spawn(move || {
            thread::sleep(Duration::from_millis(80));
            // The guard was taken on another thread: it is `Send`.
            drop(guard);
        });
        let started = Instant::now();
        let waited = lock.acquire("写入", Duration::from_secs(10)).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(60));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(lock.held_by().unwrap().operation, "写入");
        drop(waited);
        releaser.join().unwrap();
    }

    #[test]
    fn acquire_gives_up_after_the_wait_instead_of_blocking_forever() {
        let lock = KbLock::default();
        let _long_job = lock.try_acquire("入库").unwrap();
        let started = Instant::now();
        let busy = lock
            .acquire("写入", Duration::from_millis(120))
            .err()
            .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(busy.operation, "入库");
        assert!(busy.held_for >= Duration::from_millis(100));
    }

    #[test]
    fn a_panicking_holder_releases_the_lock() {
        let lock = KbLock::default();
        let clone = lock.clone();
        let result = thread::spawn(move || {
            let _guard = clone.try_acquire("同步").unwrap();
            panic!("the operation failed badly");
        })
        .join();
        assert!(result.is_err());
        assert!(lock.try_acquire("写入").is_ok());
    }

    #[test]
    fn waiters_all_get_their_turn() {
        let lock = KbLock::default();
        let first = lock.try_acquire("同步").unwrap();
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let lock = lock.clone();
                thread::spawn(move || {
                    let _guard = lock.acquire("写入", Duration::from_secs(10)).unwrap();
                    thread::sleep(Duration::from_millis(5));
                })
            })
            .collect();
        thread::sleep(Duration::from_millis(30));
        drop(first);
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(lock.held_by(), None);
    }
}
