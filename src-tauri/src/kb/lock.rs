//! The knowledge-base operation lock.
//!
//! Only one operation at a time may change the knowledge base's files or git
//! state: a write, a capture, a sync, or a long-running job that works on the
//! folder for minutes. Whoever needs the knowledge base asks for the lock and
//! says what it is doing; everyone else either waits a bounded time or gets a
//! [`Busy`] answer that names the operation in the way. Nothing ever blocks
//! forever.
//!
//! A **job** is an operation that takes minutes: an AI run (入库, 落裁决),
//! from its start until its result is committed, discarded or kept. While a
//! job runs ([`KbLock::start_job`]):
//!
//! - writes and syncs ([`KbLock::acquire`]) are refused at once with a
//!   [`Busy`] that names the job, instead of waiting for minutes;
//! - quick captures and the job's own short git steps still take the lock in
//!   turn ([`KbLock::acquire_beside_job`]), so they never overlap;
//! - commits made beside the job are noted ([`KbLockGuard::note_commit`]), so
//!   the job can tell them apart from commits nobody was supposed to make.
//!
//! The guards are `Send`, so a long-running job can take the lock on one
//! thread and release it on another. Dropping a guard (also during a panic)
//! releases what it holds.

use std::fmt;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// A cloneable handle to the one lock of the app (clones share the lock).
#[derive(Clone, Default)]
pub struct KbLock(Arc<Inner>);

#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    released: Condvar,
}

#[derive(Default)]
struct State {
    /// The operation working on the knowledge base right now.
    holder: Option<Holder>,
    /// The long-running job, if one is running.
    job: Option<Job>,
    /// Numbers the jobs, so a stale [`KbJob`] can never end a newer job.
    #[cfg_attr(not(test), allow(dead_code))]
    jobs_started: u64,
}

struct Holder {
    operation: String,
    since: Instant,
}

#[cfg_attr(not(test), allow(dead_code))]
struct Job {
    id: u64,
    operation: String,
    since: Instant,
    commits: Vec<NotedCommit>,
}

/// A commit another operation made while a job was running.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NotedCommit {
    pub commit: String,
    /// The one file the commit contains (repo-relative).
    pub path: String,
}

/// Holds the lock until dropped.
#[must_use = "the lock is released as soon as the guard is dropped"]
pub struct KbLockGuard {
    lock: KbLock,
}

/// Marks a job as running until dropped.
#[cfg_attr(not(test), allow(dead_code))]
#[must_use = "the job ends as soon as this is dropped"]
pub struct KbJob {
    lock: KbLock,
    id: u64,
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
    /// Refused at once while a job is running.
    pub fn acquire(&self, operation: &str, wait: Duration) -> Result<KbLockGuard, Busy> {
        self.take(operation, wait, false)
    }

    /// Like [`KbLock::acquire`], but also while a job is running: for quick
    /// captures, which must keep working during a job, and for the job's own
    /// short steps.
    pub fn acquire_beside_job(
        &self,
        operation: &str,
        wait: Duration,
    ) -> Result<KbLockGuard, Busy> {
        self.take(operation, wait, true)
    }

    fn take(&self, operation: &str, wait: Duration, beside_job: bool) -> Result<KbLockGuard, Busy> {
        let deadline = Instant::now() + wait;
        let mut state = self.state();
        loop {
            if !beside_job {
                if let Some(job) = &state.job {
                    return Err(job.busy());
                }
            }
            let Some(current) = state.holder.as_ref() else {
                state.holder = Some(Holder {
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
            state = self.wait(state, deadline - now);
        }
    }

    /// Starts a job called `operation`, waiting up to `wait` for the current
    /// holder to finish first. Refused at once while another job is running.
    /// The job runs until the returned [`KbJob`] is dropped; it does not hold
    /// the lock itself (see [`KbLock::acquire_beside_job`]).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn start_job(&self, operation: &str, wait: Duration) -> Result<KbJob, Busy> {
        let deadline = Instant::now() + wait;
        let mut state = self.state();
        loop {
            if let Some(job) = &state.job {
                return Err(job.busy());
            }
            let Some(current) = state.holder.as_ref() else {
                state.jobs_started += 1;
                let id = state.jobs_started;
                state.job = Some(Job {
                    id,
                    operation: operation.to_string(),
                    since: Instant::now(),
                    commits: Vec::new(),
                });
                return Ok(KbJob {
                    lock: self.clone(),
                    id,
                });
            };
            let now = Instant::now();
            if now >= deadline {
                return Err(Busy {
                    operation: current.operation.clone(),
                    held_for: now.duration_since(current.since),
                });
            }
            state = self.wait(state, deadline - now);
        }
    }

    /// What keeps the knowledge base busy at the moment, if anything: the
    /// operation holding the lock, else the running job.
    pub fn held_by(&self) -> Option<Busy> {
        let state = self.state();
        match state.holder.as_ref() {
            Some(current) => Some(Busy {
                operation: current.operation.clone(),
                held_for: current.since.elapsed(),
            }),
            None => state.job.as_ref().map(Job::busy),
        }
    }

    // The inner mutex only guards a few assignments, so a poisoned mutex still
    // holds consistent data.
    fn state(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, state: MutexGuard<'a, State>, timeout: Duration) -> MutexGuard<'a, State> {
        self.0
            .released
            .wait_timeout(state, timeout)
            .unwrap_or_else(PoisonError::into_inner)
            .0
    }
}

impl Job {
    fn busy(&self) -> Busy {
        Busy {
            operation: self.operation.clone(),
            held_for: self.since.elapsed(),
        }
    }
}

impl KbLockGuard {
    /// Tells the running job (if any) about a commit this operation made:
    /// `commit` contains exactly the file `path`.
    pub fn note_commit(&self, commit: &str, path: &str) {
        if let Some(job) = self.lock.state().job.as_mut() {
            job.commits.push(NotedCommit {
                commit: commit.to_string(),
                path: path.to_string(),
            });
        }
    }
}

impl Drop for KbLockGuard {
    fn drop(&mut self) {
        self.lock.state().holder = None;
        self.lock.0.released.notify_all();
    }
}

impl KbJob {
    /// Changes what the job says it is doing (shown in "busy" messages).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn set_operation(&self, operation: &str) {
        let mut state = self.lock.state();
        if let Some(job) = state.job.as_mut().filter(|job| job.id == self.id) {
            job.operation = operation.to_string();
        }
    }

    /// The commits other operations made since the job started, oldest first.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn noted_commits(&self) -> Vec<NotedCommit> {
        let state = self.lock.state();
        state
            .job
            .as_ref()
            .filter(|job| job.id == self.id)
            .map(|job| job.commits.clone())
            .unwrap_or_default()
    }
}

impl Drop for KbJob {
    fn drop(&mut self) {
        let mut state = self.lock.state();
        if state.job.as_ref().is_some_and(|job| job.id == self.id) {
            state.job = None;
        }
        drop(state);
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

    #[test]
    fn a_job_refuses_writes_and_syncs_at_once_but_lets_captures_through() {
        let lock = KbLock::default();
        let job = lock.start_job("入库", Duration::ZERO).unwrap();
        assert_eq!(lock.held_by().unwrap().operation, "入库");

        // A write would normally wait: during a job it is told at once.
        let started = Instant::now();
        let busy = lock.acquire("写入", Duration::from_secs(30)).err().unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(busy.operation, "入库");
        assert!(busy.to_string().contains("正在入库"), "{busy}");
        assert_eq!(lock.try_acquire("同步").err().unwrap().operation, "入库");

        // A capture takes the lock in turn, and so does the job itself.
        let capture = lock
            .acquire_beside_job("写入", Duration::from_secs(5))
            .unwrap();
        assert_eq!(lock.held_by().unwrap().operation, "写入");
        let second = lock
            .acquire_beside_job("入库", Duration::from_millis(60))
            .err()
            .unwrap();
        assert_eq!(second.operation, "写入");
        drop(capture);
        drop(lock.acquire_beside_job("入库", Duration::ZERO).unwrap());

        drop(job);
        assert_eq!(lock.held_by(), None);
        let _write = lock.try_acquire("写入").unwrap();
    }

    #[test]
    fn only_one_job_at_a_time_and_it_waits_for_the_holder() {
        let lock = KbLock::default();
        let holder = lock.try_acquire("同步").unwrap();
        assert_eq!(
            lock.start_job("入库", Duration::from_millis(60))
                .err()
                .unwrap()
                .operation,
            "同步"
        );
        let releaser = thread::spawn(move || {
            thread::sleep(Duration::from_millis(80));
            drop(holder);
        });
        let job = lock.start_job("入库", Duration::from_secs(10)).unwrap();
        releaser.join().unwrap();

        let started = Instant::now();
        let busy = lock.start_job("落裁决", Duration::from_secs(30)).err().unwrap();
        assert_eq!(busy.operation, "入库");
        assert!(started.elapsed() < Duration::from_secs(5));
        drop(job);
        let _next = lock.start_job("落裁决", Duration::ZERO).unwrap();
    }

    #[test]
    fn a_job_hears_about_commits_made_beside_it_and_can_rename_itself() {
        let lock = KbLock::default();
        // Without a job there is nobody to tell.
        lock.try_acquire("写入").unwrap().note_commit("aaa", "inbox/0.md");

        let job = lock.start_job("入库", Duration::ZERO).unwrap();
        assert!(job.noted_commits().is_empty());
        {
            let capture = lock.acquire_beside_job("写入", Duration::ZERO).unwrap();
            capture.note_commit("abc", "inbox/1.md");
        }
        lock.acquire_beside_job("写入", Duration::ZERO)
            .unwrap()
            .note_commit("def", "inbox/2.md");
        assert_eq!(
            job.noted_commits(),
            vec![
                NotedCommit {
                    commit: "abc".into(),
                    path: "inbox/1.md".into()
                },
                NotedCommit {
                    commit: "def".into(),
                    path: "inbox/2.md".into()
                },
            ]
        );

        job.set_operation("等你处理入库结果");
        assert_eq!(
            lock.try_acquire("同步").err().unwrap().operation,
            "等你处理入库结果"
        );
    }

    #[test]
    fn a_job_ends_when_its_owner_panics_and_stale_jobs_end_nothing() {
        let lock = KbLock::default();
        let clone = lock.clone();
        let result = thread::spawn(move || {
            let _job = clone.start_job("入库", Duration::ZERO).unwrap();
            panic!("the run failed badly");
        })
        .join();
        assert!(result.is_err());
        assert_eq!(lock.held_by(), None);

        let first = lock.start_job("入库", Duration::ZERO).unwrap();
        let first_again = KbJob {
            lock: lock.clone(),
            id: first.id,
        };
        drop(first);
        let second = lock.start_job("落裁决", Duration::ZERO).unwrap();
        // A leftover handle of the first job must not end the second one.
        first_again.set_operation("不该生效");
        drop(first_again);
        assert_eq!(lock.held_by().unwrap().operation, "落裁决");
        drop(second);
    }
}
