use std::ffi::{CStr, CString};
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Error};
use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::sys::stat::Mode;

use pxar::Metadata;

use proxmox_sys::c_result;
use proxmox_sys::error::SysError;
use proxmox_sys::fs::{self, acl, xattr};

use crate::pxar::tools::perms_from_metadata;
use crate::pxar::Flags;

//
// utility functions
//

fn allow_notsupp<E: SysError>(err: E) -> Result<(), E> {
    if err.is_errno(Errno::EOPNOTSUPP) {
        Ok(())
    } else {
        Err(err)
    }
}

fn allow_notsupp_remember<E: SysError>(err: E, not_supp: &mut bool) -> Result<(), E> {
    if err.is_errno(Errno::EOPNOTSUPP) {
        *not_supp = true;
        Ok(())
    } else {
        Err(err)
    }
}

fn timestamp_to_update_timespec(mtime: &pxar::format::StatxTimestamp) -> [libc::timespec; 2] {
    // restore mtime
    const UTIME_OMIT: i64 = (1 << 30) - 2;

    [
        libc::timespec {
            tv_sec: 0,
            tv_nsec: UTIME_OMIT,
        },
        libc::timespec {
            tv_sec: mtime.secs,
            tv_nsec: mtime.nanos as _,
        },
    ]
}

//
// metadata application:
//

pub fn apply_at(
    flags: Flags,
    metadata: &Metadata,
    parent: RawFd,
    file_name: &CStr,
    path_info: &Path,
    on_error: &mut (dyn FnMut(Error) -> Result<(), Error> + Send),
) -> Result<(), Error> {
    let fd = proxmox_sys::fd::openat(
        &parent,
        file_name,
        OFlag::O_PATH | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    )?;

    apply(flags, metadata, fd.as_raw_fd(), path_info, on_error)
}

pub fn apply_initial_flags(
    flags: Flags,
    metadata: &Metadata,
    fd: RawFd,
    on_error: &mut (dyn FnMut(Error) -> Result<(), Error> + Send),
) -> Result<(), Error> {
    let entry_flags = Flags::from_bits_truncate(metadata.stat.flags);
    apply_chattr(
        fd,
        entry_flags.to_initial_chattr(),
        flags.to_initial_chattr(),
    )
    .or_else(on_error)?;
    Ok(())
}

pub fn apply(
    flags: Flags,
    metadata: &Metadata,
    fd: RawFd,
    path_info: &Path,
    on_error: &mut (dyn FnMut(Error) -> Result<(), Error> + Send),
) -> Result<(), Error> {
    let c_proc_path = CString::new(format!("/proc/self/fd/{}", fd)).unwrap();
    apply_ownership(flags, c_proc_path.as_ptr(), metadata, &mut *on_error)?;

    let mut skip_xattrs = false;
    apply_xattrs(flags, c_proc_path.as_ptr(), metadata, &mut skip_xattrs)
        .or_else(&mut *on_error)?;
    add_fcaps(flags, c_proc_path.as_ptr(), metadata, &mut skip_xattrs).or_else(&mut *on_error)?;
    apply_acls(flags, &c_proc_path, metadata, path_info)
        .context("failed to apply acls")
        .or_else(&mut *on_error)?;
    apply_quota_project_id(flags, fd, metadata).or_else(&mut *on_error)?;

    // Finally mode and time. We may lose access with mode, but the changing the mode also
    // affects times.
    if !metadata.is_symlink() && flags.contains(Flags::WITH_PERMISSIONS) {
        c_result!(unsafe {
            libc::chmod(c_proc_path.as_ptr(), perms_from_metadata(metadata)?.bits())
        })
        .map(drop)
        .or_else(allow_notsupp)
        .context("failed to change file mode")
        .or_else(&mut *on_error)?;
    }

    let res = c_result!(unsafe {
        libc::utimensat(
            libc::AT_FDCWD,
            c_proc_path.as_ptr(),
            timestamp_to_update_timespec(&metadata.stat.mtime).as_ptr(),
            0,
        )
    });
    match res {
        Ok(_) => (),
        Err(ref err) if err.is_errno(Errno::EOPNOTSUPP) => (),
        Err(err) => {
            on_error(anyhow!(err).context(format!(
                "failed to restore mtime attribute on {path_info:?}"
            )))?;
        }
    }

    if metadata.stat.flags != 0 {
        apply_flags(flags, fd, metadata.stat.flags).or_else(&mut *on_error)?;
    }

    Ok(())
}

pub fn apply_ownership(
    flags: Flags,
    c_proc_path: *const libc::c_char,
    metadata: &Metadata,
    on_error: &mut (dyn FnMut(Error) -> Result<(), Error> + Send),
) -> Result<(), Error> {
    if !flags.contains(Flags::WITH_OWNER) {
        return Ok(());
    }
    unsafe {
        // UID and GID first, as this fails if we lose access anyway.
        c_result!(libc::chown(
            c_proc_path,
            metadata.stat.uid,
            metadata.stat.gid
        ))
        .map(drop)
        .or_else(allow_notsupp)
        .context("failed to set ownership")
        .or_else(&mut *on_error)?;
    }
    Ok(())
}

fn add_fcaps(
    flags: Flags,
    c_proc_path: *const libc::c_char,
    metadata: &Metadata,
    skip_xattrs: &mut bool,
) -> Result<(), Error> {
    if *skip_xattrs || !flags.contains(Flags::WITH_FCAPS) {
        return Ok(());
    }
    let fcaps = match metadata.fcaps.as_ref() {
        Some(fcaps) => fcaps,
        None => return Ok(()),
    };

    c_result!(unsafe {
        libc::setxattr(
            c_proc_path,
            xattr::xattr_name_fcaps().as_ptr(),
            fcaps.data.as_ptr() as *const libc::c_void,
            fcaps.data.len(),
            0,
        )
    })
    .map(drop)
    .or_else(|err| allow_notsupp_remember(err, skip_xattrs))
    .context("failed to apply file capabilities")
}

fn apply_xattrs(
    flags: Flags,
    c_proc_path: *const libc::c_char,
    metadata: &Metadata,
    skip_xattrs: &mut bool,
) -> Result<(), Error> {
    if *skip_xattrs || !flags.contains(Flags::WITH_XATTRS) {
        return Ok(());
    }

    for xattr in &metadata.xattrs {
        if *skip_xattrs {
            return Ok(());
        }

        if !xattr::is_valid_xattr_name(xattr.name()) {
            log::info!("skipping invalid xattr named {:?}", xattr.name());
            continue;
        }

        c_result!(unsafe {
            libc::setxattr(
                c_proc_path,
                xattr.name().as_ptr() as *const libc::c_char,
                xattr.value().as_ptr() as *const libc::c_void,
                xattr.value().len(),
                0,
            )
        })
        .map(drop)
        .or_else(|err| allow_notsupp_remember(err, &mut *skip_xattrs))
        .context("failed to apply extended attributes")?;
    }

    Ok(())
}

fn apply_acls(
    flags: Flags,
    c_proc_path: &CStr,
    metadata: &Metadata,
    path_info: &Path,
) -> Result<(), Error> {
    if !flags.contains(Flags::WITH_ACL) || metadata.acl.is_empty() {
        return Ok(());
    }

    let mut acl = acl::ACL::init(5)?;

    // acl type access:
    acl.add_entry_full(
        acl::ACL_USER_OBJ,
        None,
        acl::mode_user_to_acl_permissions(metadata.stat.mode),
    )?;

    acl.add_entry_full(
        acl::ACL_OTHER,
        None,
        acl::mode_other_to_acl_permissions(metadata.stat.mode),
    )?;

    match metadata.acl.group_obj.as_ref() {
        Some(group_obj) => {
            acl.add_entry_full(
                acl::ACL_MASK,
                None,
                acl::mode_group_to_acl_permissions(metadata.stat.mode),
            )?;
            acl.add_entry_full(acl::ACL_GROUP_OBJ, None, group_obj.permissions.0)?;
        }
        None => {
            let mode = acl::mode_group_to_acl_permissions(metadata.stat.mode);

            acl.add_entry_full(acl::ACL_GROUP_OBJ, None, mode)?;

            if !metadata.acl.users.is_empty() || !metadata.acl.groups.is_empty() {
                log::warn!(
                    "Warning: {:?}: Missing GROUP_OBJ entry in ACL, resetting to value of MASK",
                    path_info,
                );
                acl.add_entry_full(acl::ACL_MASK, None, mode)?;
            }
        }
    }

    for user in &metadata.acl.users {
        acl.add_entry_full(acl::ACL_USER, Some(user.uid), user.permissions.0)?;
    }

    for group in &metadata.acl.groups {
        acl.add_entry_full(acl::ACL_GROUP, Some(group.gid), group.permissions.0)?;
    }

    if !acl.is_valid() {
        bail!("Error while restoring ACL - ACL invalid");
    }

    acl.set_file(c_proc_path, acl::ACL_TYPE_ACCESS)?;
    drop(acl);

    // acl type default:
    if let Some(default) = metadata.acl.default.as_ref() {
        let mut acl = acl::ACL::init(5)?;

        acl.add_entry_full(acl::ACL_USER_OBJ, None, default.user_obj_permissions.0)?;

        acl.add_entry_full(acl::ACL_GROUP_OBJ, None, default.group_obj_permissions.0)?;

        acl.add_entry_full(acl::ACL_OTHER, None, default.other_permissions.0)?;

        if default.mask_permissions != pxar::format::acl::Permissions::NO_MASK {
            acl.add_entry_full(acl::ACL_MASK, None, default.mask_permissions.0)?;
        }

        for user in &metadata.acl.default_users {
            acl.add_entry_full(acl::ACL_USER, Some(user.uid), user.permissions.0)?;
        }

        for group in &metadata.acl.default_groups {
            acl.add_entry_full(acl::ACL_GROUP, Some(group.gid), group.permissions.0)?;
        }

        if !acl.is_valid() {
            bail!("Error while restoring ACL - ACL invalid");
        }

        acl.set_file(c_proc_path, acl::ACL_TYPE_DEFAULT)?;
    }

    Ok(())
}

fn apply_quota_project_id(flags: Flags, fd: RawFd, metadata: &Metadata) -> Result<(), Error> {
    if !flags.contains(Flags::WITH_QUOTA_PROJID) {
        return Ok(());
    }

    let projid = match metadata.quota_project_id {
        Some(projid) => projid,
        None => return Ok(()),
    };

    let mut fsxattr = fs::FSXAttr::default();
    unsafe {
        fs::fs_ioc_fsgetxattr(fd, &mut fsxattr)
            .context("error while getting fsxattr to restore quota project id")?;

        fsxattr.fsx_projid = projid.projid as u32;

        fs::fs_ioc_fssetxattr(fd, &fsxattr)
            .context("error while setting fsxattr to restore quota project id")?;
    }

    Ok(())
}

pub(crate) fn errno_is_unsupported(errno: Errno) -> bool {
    matches!(
        errno,
        Errno::ENOTTY | Errno::ENOSYS | Errno::EBADF | Errno::EOPNOTSUPP | Errno::EINVAL
    )
}

fn apply_chattr(fd: RawFd, chattr: libc::c_long, mask: libc::c_long) -> Result<(), Error> {
    if chattr == 0 {
        return Ok(());
    }

    let mut fattr: libc::c_long = 0;
    match unsafe { fs::read_attr_fd(fd, &mut fattr) } {
        Ok(_) => (),
        Err(errno) if errno_is_unsupported(errno) => {
            return Ok(());
        }
        Err(err) => return Err(err).context("failed to read file attributes"),
    }

    let attr = (chattr & mask) | (fattr & !mask);

    if attr == fattr {
        return Ok(());
    }

    match unsafe { fs::write_attr_fd(fd, &attr) } {
        Ok(_) => Ok(()),
        Err(errno) if errno_is_unsupported(errno) => Ok(()),
        Err(err) => Err(err).context("failed to set file attributes"),
    }
}

fn apply_flags(flags: Flags, fd: RawFd, entry_flags: u64) -> Result<(), Error> {
    let entry_flags = Flags::from_bits_truncate(entry_flags);

    apply_chattr(fd, entry_flags.to_chattr(), flags.to_chattr())?;

    let fatattr = (flags & entry_flags).to_fat_attr();
    if fatattr != 0 {
        match unsafe { fs::write_fat_attr_fd(fd, &fatattr) } {
            Ok(_) => (),
            Err(errno) if errno_is_unsupported(errno) => (),
            Err(err) => return Err(err).context("failed to set file FAT attributes"),
        }
    }

    Ok(())
}
