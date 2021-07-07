//! Inter-process reader-writer lock builder.
//!
//! This implementation uses fcntl record locks with non-blocking
//! F_SETLK command (never blocks).
//!
//! We maintain a map of shared locks with time stamps, so you can get
//! the timestamp for the oldest open lock with
//! `oldest_shared_lock()`.

use std::collections::HashMap;
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Error};

// fixme: use F_OFD_ locks when implemented with nix::fcntl

// Note: flock lock conversion is not atomic, so we need to use fcntl

/// Inter-process reader-writer lock
pub struct ProcessLocker {
    file: std::fs::File,
    exclusive: bool,
    writers: usize,
    next_guard_id: u64,
    shared_guard_list: HashMap<u64, i64>, // guard_id => timestamp
}

/// Lock guard for shared locks
///
/// Release the lock when it goes out of scope.
pub struct ProcessLockSharedGuard {
    guard_id: u64,
    locker: Arc<Mutex<ProcessLocker>>,
}

impl Drop for ProcessLockSharedGuard {
    fn drop(&mut self) {
        let mut data = self.locker.lock().unwrap();

        if data.writers == 0 {
            panic!("unexpected ProcessLocker state");
        }

        data.shared_guard_list.remove(&self.guard_id);

        if data.writers == 1 && !data.exclusive {
            let op = libc::flock {
                l_type: libc::F_UNLCK as i16,
                l_whence: libc::SEEK_SET as i16,
                l_start: 0,
                l_len: 0,
                l_pid: 0,
            };

            if let Err(err) =
                nix::fcntl::fcntl(data.file.as_raw_fd(), nix::fcntl::FcntlArg::F_SETLKW(&op))
            {
                panic!("unable to drop writer lock - {}", err);
            }
        }
        if data.writers > 0 {
            data.writers -= 1;
        }
    }
}

/// Lock guard for exclusive locks
///
/// Release the lock when it goes out of scope.
pub struct ProcessLockExclusiveGuard {
    locker: Arc<Mutex<ProcessLocker>>,
}

impl Drop for ProcessLockExclusiveGuard {
    fn drop(&mut self) {
        let mut data = self.locker.lock().unwrap();

        if !data.exclusive {
            panic!("unexpected ProcessLocker state");
        }

        let ltype = if data.writers != 0 {
            libc::F_RDLCK
        } else {
            libc::F_UNLCK
        };
        let op = libc::flock {
            l_type: ltype as i16,
            l_whence: libc::SEEK_SET as i16,
            l_start: 0,
            l_len: 0,
            l_pid: 0,
        };

        if let Err(err) =
            nix::fcntl::fcntl(data.file.as_raw_fd(), nix::fcntl::FcntlArg::F_SETLKW(&op))
        {
            panic!("unable to drop exclusive lock - {}", err);
        }

        data.exclusive = false;
    }
}

impl ProcessLocker {
    /// Create a new instance for the specified file.
    ///
    /// This simply creates the file if it does not exist.
    pub fn new<P: AsRef<std::path::Path>>(lockfile: P) -> Result<Arc<Mutex<Self>>, Error> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(lockfile)?;

        Ok(Arc::new(Mutex::new(Self {
            file,
            exclusive: false,
            writers: 0,
            next_guard_id: 0,
            shared_guard_list: HashMap::new(),
        })))
    }

    fn try_lock(file: &std::fs::File, ltype: i32) -> Result<(), Error> {
        let op = libc::flock {
            l_type: ltype as i16,
            l_whence: libc::SEEK_SET as i16,
            l_start: 0,
            l_len: 0,
            l_pid: 0,
        };

        nix::fcntl::fcntl(file.as_raw_fd(), nix::fcntl::FcntlArg::F_SETLK(&op))?;

        Ok(())
    }

    /// Try to acquire a shared lock
    ///
    /// On success, this makes sure that no other process can get an exclusive lock for the file.
    pub fn try_shared_lock(locker: Arc<Mutex<Self>>) -> Result<ProcessLockSharedGuard, Error> {
        let mut data = locker.lock().unwrap();

        if data.writers == 0 && !data.exclusive {
            if let Err(err) = Self::try_lock(&data.file, libc::F_RDLCK) {
                bail!("unable to get shared lock - {}", err);
            }
        }

        data.writers += 1;

        let guard = ProcessLockSharedGuard {
            locker: locker.clone(),
            guard_id: data.next_guard_id,
        };
        data.next_guard_id += 1;

        let now = unsafe { libc::time(std::ptr::null_mut()) };

        data.shared_guard_list.insert(guard.guard_id, now);

        Ok(guard)
    }

    /// Get oldest shared lock timestamp
    pub fn oldest_shared_lock(locker: Arc<Mutex<Self>>) -> Option<i64> {
        let mut result = None;

        let data = locker.lock().unwrap();

        for v in data.shared_guard_list.values() {
            result = match result {
                None => Some(*v),
                Some(x) => {
                    if x < *v {
                        Some(x)
                    } else {
                        Some(*v)
                    }
                }
            };
        }

        result
    }

    /// Try to acquire a exclusive lock
    ///
    /// Make sure the we are the only process which has locks for this file (shared or exclusive).
    pub fn try_exclusive_lock(
        locker: Arc<Mutex<Self>>,
    ) -> Result<ProcessLockExclusiveGuard, Error> {
        let mut data = locker.lock().unwrap();

        if data.exclusive {
            bail!("already locked exclusively");
        }

        if let Err(err) = Self::try_lock(&data.file, libc::F_WRLCK) {
            bail!("unable to get exclusive lock - {}", err);
        }

        data.exclusive = true;

        Ok(ProcessLockExclusiveGuard {
            locker: locker.clone(),
        })
    }
}
