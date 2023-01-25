use std::mem::MaybeUninit;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Error};
use nix::sys::stat::Mode;

use proxmox_sys::fs::{create_path, CreateOptions};

use proxmox_http::{RateLimit, RateLimiter, ShareableRateLimit};
use proxmox_shared_memory::{check_subtype, initialize_subtype};
use proxmox_shared_memory::{Init, SharedMemory, SharedMutex};

// openssl::sha::sha256(b"Proxmox Backup SharedRateLimiter v1.0")[0..8];
pub const PROXMOX_BACKUP_SHARED_RATE_LIMITER_MAGIC_1_0: [u8; 8] =
    [6, 58, 213, 96, 161, 122, 130, 117];

const BASE_PATH: &str = pbs_buildcfg::rundir!("/shmem/tbf");

// Wrap RateLimiter, so that we can provide an Init impl
#[repr(C)]
struct WrapLimiter(RateLimiter);

impl Init for WrapLimiter {
    fn initialize(this: &mut MaybeUninit<Self>) {
        // default does not matter here, because we override later
        this.write(WrapLimiter(RateLimiter::new(1_000_000, 1_000_000)));
    }
}

#[repr(C)]
struct SharedRateLimiterData {
    magic: [u8; 8],
    tbf: SharedMutex<WrapLimiter>,
    padding: [u8; 4096 - 104],
}

impl Init for SharedRateLimiterData {
    fn initialize(this: &mut MaybeUninit<Self>) {
        unsafe {
            let me = &mut *this.as_mut_ptr();
            me.magic = PROXMOX_BACKUP_SHARED_RATE_LIMITER_MAGIC_1_0;
            initialize_subtype(&mut me.tbf);
        }
    }

    fn check_type_magic(this: &MaybeUninit<Self>) -> Result<(), Error> {
        unsafe {
            let me = &*this.as_ptr();
            if me.magic != PROXMOX_BACKUP_SHARED_RATE_LIMITER_MAGIC_1_0 {
                bail!("SharedRateLimiterData: wrong magic number");
            }
            check_subtype(&me.tbf)?;
            Ok(())
        }
    }
}

/// Rate limiter designed for shared memory ([SharedMemory])
///
/// The actual [RateLimiter] is protected by a [SharedMutex] and
/// implements [Init]. This way we can share the limiter between
/// different processes.
pub struct SharedRateLimiter {
    shmem: SharedMemory<SharedRateLimiterData>,
}

impl SharedRateLimiter {
    /// Creates a new mmap'ed instance.
    ///
    /// Data is mapped in `/var/run/proxmox-backup/shmem/tbf/<name>` using
    /// `TMPFS`.
    pub fn mmap_shmem(name: &str, rate: u64, burst: u64) -> Result<Self, Error> {
        let mut path = PathBuf::from(BASE_PATH);

        let user = pbs_config::backup_user()?;

        let dir_opts = CreateOptions::new()
            .perm(Mode::from_bits_truncate(0o770))
            .owner(user.uid)
            .group(user.gid);

        create_path(&path, Some(dir_opts.clone()), Some(dir_opts))?;

        path.push(name);

        let file_opts = CreateOptions::new()
            .perm(Mode::from_bits_truncate(0o660))
            .owner(user.uid)
            .group(user.gid);

        let shmem: SharedMemory<SharedRateLimiterData> = SharedMemory::open(&path, file_opts)?;

        shmem.data().tbf.lock().0.update_rate(rate, burst);

        Ok(Self { shmem })
    }
}

impl ShareableRateLimit for SharedRateLimiter {
    fn update_rate(&self, rate: u64, bucket_size: u64) {
        self.shmem
            .data()
            .tbf
            .lock()
            .0
            .update_rate(rate, bucket_size);
    }

    fn traffic(&self) -> u64 {
        self.shmem.data().tbf.lock().0.traffic()
    }

    fn register_traffic(&self, current_time: Instant, data_len: u64) -> Duration {
        self.shmem
            .data()
            .tbf
            .lock()
            .0
            .register_traffic(current_time, data_len)
    }
}
