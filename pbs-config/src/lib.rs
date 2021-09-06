pub mod domains;
pub mod drive;
pub mod media_pool;
pub mod remote;

use anyhow::{format_err, Error};

pub use pbs_buildcfg::{BACKUP_USER_NAME, BACKUP_GROUP_NAME};

/// Return User info for the 'backup' user (``getpwnam_r(3)``)
pub fn backup_user() -> Result<nix::unistd::User, Error> {
    pbs_tools::sys::query_user(BACKUP_USER_NAME)?
        .ok_or_else(|| format_err!("Unable to lookup '{}' user.", BACKUP_USER_NAME))
}

/// Return Group info for the 'backup' group (``getgrnam(3)``)
pub fn backup_group() -> Result<nix::unistd::Group, Error> {
    pbs_tools::sys::query_group(BACKUP_GROUP_NAME)?
        .ok_or_else(|| format_err!("Unable to lookup '{}' group.", BACKUP_GROUP_NAME))
}
pub struct BackupLockGuard(Option<std::fs::File>);

#[doc(hidden)]
/// Note: do not use for production code, this is only intended for tests
pub unsafe fn create_mocked_lock() -> BackupLockGuard {
    BackupLockGuard(None)
}

/// Open or create a lock file owned by user "backup" and lock it.
///
/// Owner/Group of the file is set to backup/backup.
/// File mode is 0660.
/// Default timeout is 10 seconds.
///
/// Note: This method needs to be called by user "root" or "backup".
pub fn open_backup_lockfile<P: AsRef<std::path::Path>>(
    path: P,
    timeout: Option<std::time::Duration>,
    exclusive: bool,
) -> Result<BackupLockGuard, Error> {
    let user = backup_user()?;
    let options = proxmox::tools::fs::CreateOptions::new()
        .perm(nix::sys::stat::Mode::from_bits_truncate(0o660))
        .owner(user.uid)
        .group(user.gid);

    let timeout = timeout.unwrap_or(std::time::Duration::new(10, 0));

    let file = proxmox::tools::fs::open_file_locked(&path, timeout, exclusive, options)?;
    Ok(BackupLockGuard(Some(file)))
}

/// Atomically write data to file owned by "root:backup" with permission "0640"
///
/// Only the superuser can write those files, but group 'backup' can read them.
pub fn replace_backup_config<P: AsRef<std::path::Path>>(
    path: P,
    data: &[u8],
) -> Result<(), Error> {
    let backup_user = backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);
    // set the correct owner/group/permissions while saving file
    // owner(rw) = root, group(r)= backup
    let options = proxmox::tools::fs::CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(backup_user.gid);

    proxmox::tools::fs::replace_file(path, data, options)?;

    Ok(())
}

/// Atomically write data to file owned by "root:root" with permission "0600"
///
/// Only the superuser can read and write those files.
pub fn replace_secret_config<P: AsRef<std::path::Path>>(
    path: P,
    data: &[u8],
) -> Result<(), Error> {
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);
    // set the correct owner/group/permissions while saving file
    // owner(rw) = root, group(r)= root
    let options = proxmox::tools::fs::CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(nix::unistd::Gid::from_raw(0));

    proxmox::tools::fs::replace_file(path, data, options)?;

    Ok(())
}
