//! Memory based communication channel between proxy & daemon for things such as cache
//! invalidation.

use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Error;
use nix::fcntl::OFlag;
use nix::sys::mman::{MapFlags, ProtFlags};
use nix::sys::stat::Mode;
use once_cell::sync::OnceCell;

use proxmox::tools::fs::CreateOptions;
use proxmox::tools::mmap::Mmap;

/// In-memory communication channel.
pub struct Memcom {
    mmap: Mmap<u8>,
}

#[repr(C)]
struct Head {
    // User (user.cfg) cache generation/version.
    user_cache_generation: AtomicUsize,
    // Traffic control (traffic-control.cfg) generation/version.
    traffic_control_generation: AtomicUsize,
}

static INSTANCE: OnceCell<Arc<Memcom>> = OnceCell::new();

const MEMCOM_FILE_PATH: &str = pbs_buildcfg::rundir!("/proxmox-backup-memcom");
const EMPTY_PAGE: [u8; 4096] = [0u8; 4096];

impl Memcom {
    /// Open the memory based communication channel singleton.
    pub fn new() -> Result<Arc<Self>, Error> {
        INSTANCE.get_or_try_init(Self::open).map(Arc::clone)
    }

    // Actual work of `new`:
    fn open() -> Result<Arc<Self>, Error> {
        let user = crate::backup_user()?;
        let options = CreateOptions::new()
            .perm(Mode::from_bits_truncate(0o660))
            .owner(user.uid)
            .group(user.gid);

        let file = proxmox::tools::fs::atomic_open_or_create_file(
            MEMCOM_FILE_PATH,
            OFlag::O_RDWR | OFlag::O_CLOEXEC,
            &EMPTY_PAGE,
            options,
            true,
        )?;

        let mmap = unsafe {
            Mmap::<u8>::map_fd(
                file.as_raw_fd(),
                0,
                4096,
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
                MapFlags::MAP_SHARED | MapFlags::MAP_NORESERVE | MapFlags::MAP_POPULATE,
            )?
        };

        Ok(Arc::new(Self { mmap }))
    }

    // Shortcut to get the mapped `Head` as a `Head`.
    fn head(&self) -> &Head {
        unsafe { &*(self.mmap.as_ptr() as *const u8 as *const Head) }
    }

    /// Returns the user cache generation number.
    pub fn user_cache_generation(&self) -> usize {
        self.head().user_cache_generation.load(Ordering::Acquire)
    }

    /// Increase the user cache generation number.
    pub fn increase_user_cache_generation(&self) {
        self.head()
            .user_cache_generation
            .fetch_add(1, Ordering::AcqRel);
    }

    /// Returns the traffic control generation number.
    pub fn traffic_control_generation(&self) -> usize {
        self.head().traffic_control_generation.load(Ordering::Acquire)
    }

    /// Increase the traffic control generation number.
    pub fn increase_traffic_control_generation(&self) {
        self.head()
            .traffic_control_generation
            .fetch_add(1, Ordering::AcqRel);
    }
}
