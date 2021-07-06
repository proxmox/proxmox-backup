//! Memory based communication channel between proxy & daemon for things such as cache
//! invalidation.

use std::ffi::CString;
use std::io;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{bail, format_err, Error};
use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::sys::mman::{MapFlags, ProtFlags};
use nix::sys::stat::Mode;
use once_cell::sync::OnceCell;

use proxmox::sys::error::SysError;
use proxmox::tools::fd::Fd;
use proxmox::tools::mmap::Mmap;

/// In-memory communication channel.
pub struct Memcom {
    mmap: Mmap<u8>,
}

#[repr(C)]
struct Head {
    // User (user.cfg) cache generation/version.
    user_cache_generation: AtomicUsize,
}

static INSTANCE: OnceCell<Arc<Memcom>> = OnceCell::new();

const MEMCOM_FILE_PATH: &str = pbs_buildcfg::rundir!("/proxmox-backup-memcom");

impl Memcom {
    /// Open the memory based communication channel singleton.
    pub fn new() -> Result<Arc<Self>, Error> {
        INSTANCE.get_or_try_init(Self::open).map(Arc::clone)
    }

    // Actual work of `new`:
    fn open() -> Result<Arc<Self>, Error> {
        let fd = match open_existing() {
            Ok(fd) => fd,
            Err(err) if err.not_found() => create_new()?,
            Err(err) => bail!("failed to open {} - {}", MEMCOM_FILE_PATH, err),
        };

        let mmap = unsafe {
            Mmap::<u8>::map_fd(
                fd.as_raw_fd(),
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
}

/// The fast path opens an existing file.
fn open_existing() -> Result<Fd, nix::Error> {
    Fd::open(
        MEMCOM_FILE_PATH,
        OFlag::O_RDWR | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
}

/// Since we need to initialize the file, we also need a solid slow path where we create the file.
/// In order to make sure the next user's `open()` vs `mmap()` race against our `truncate()` call,
/// we create it in a temporary location and rotate it in place.
fn create_new() -> Result<Fd, Error> {
    // create a temporary file:
    let temp_file_name = format!("{}.{}", MEMCOM_FILE_PATH, unsafe { libc::getpid() });
    let fd = Fd::open(
        temp_file_name.as_str(),
        OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_RDWR | OFlag::O_CLOEXEC,
        Mode::from_bits_truncate(0o660),
    )
    .map_err(|err| {
        format_err!(
            "failed to create new in-memory communication file at {} - {}",
            temp_file_name,
            err
        )
    })?;

    // let it be a page in size, it'll be initialized to zero by the kernel
    nix::unistd::ftruncate(fd.as_raw_fd(), 4096)
        .map_err(|err| format_err!("failed to set size of {} - {}", temp_file_name, err))?;

    // if this is the pbs-daemon (running as root) rather than the proxy (running as backup user),
    // make sure the backup user can access the file:
    if let Ok(backup_user) = crate::backup::backup_user() {
        match nix::unistd::fchown(fd.as_raw_fd(), None, Some(backup_user.gid)) {
            Ok(()) => (),
            Err(err) if err.is_errno(Errno::EPERM) => {
                // we're not the daemon (root), so the file is already owned by the backup user
            }
            Err(err) => bail!(
                "failed to set group to 'backup' for {} - {}",
                temp_file_name,
                err
            ),
        }
    }

    // rotate the file into place, but use `RENAME_NOREPLACE`, so in case 2 processes race against
    // the initialization, the first one wins!
    // TODO: nicer `renameat2()` wrapper in `proxmox::sys`?
    let c_file_name = CString::new(temp_file_name.as_bytes()).unwrap();
    let new_path = CString::new(MEMCOM_FILE_PATH).unwrap();
    let rc = unsafe {
        libc::renameat2(
            -1,
            c_file_name.as_ptr(),
            -1,
            new_path.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc == 0 {
        return Ok(fd);
    }
    let err = io::Error::last_os_error();

    // if another process has already raced ahead and created the file, let's just open theirs
    // instead:
    if err.kind() == io::ErrorKind::AlreadyExists {
        // someone beat us to it:
        drop(fd);
        return open_existing().map_err(Error::from);
    }

    // for any other errors, just bail out
    bail!(
        "failed to move file at {} into place at {} - {}",
        temp_file_name,
        MEMCOM_FILE_PATH,
        err
    );
}
