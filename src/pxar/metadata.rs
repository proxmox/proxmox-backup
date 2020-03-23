use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::path::Path;

use anyhow::{bail, format_err, Error};
use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::sys::stat::Mode;

use pxar::Metadata;

use proxmox::sys::error::SysError;
use proxmox::tools::fd::RawFdNum;
use proxmox::{c_result, c_try};

use crate::pxar::flags;
use crate::pxar::tools::perms_from_metadata;
use crate::tools::{acl, fs, xattr};

//
// utility functions
//

fn flags_contain(flags: u64, test_flag: u64) -> bool {
    0 != (flags & test_flag)
}

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

fn nsec_to_update_timespec(mtime_nsec: u64) -> [libc::timespec; 2] {
    // restore mtime
    const UTIME_OMIT: i64 = (1 << 30) - 2;
    const NANOS_PER_SEC: i64 = 1_000_000_000;

    let sec = (mtime_nsec as i64) / NANOS_PER_SEC;
    let nsec = (mtime_nsec as i64) % NANOS_PER_SEC;

    let times: [libc::timespec; 2] = [
        libc::timespec {
            tv_sec: 0,
            tv_nsec: UTIME_OMIT,
        },
        libc::timespec {
            tv_sec: sec,
            tv_nsec: nsec,
        },
    ];

    times
}

//
// metadata application:
//

pub fn apply_at(
    flags: u64,
    metadata: &Metadata,
    parent: RawFd,
    file_name: &CStr,
) -> Result<(), Error> {
    let fd = proxmox::tools::fd::Fd::openat(
        &unsafe { RawFdNum::from_raw_fd(parent) },
        file_name,
        OFlag::O_PATH | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    )?;

    apply(flags, metadata, fd.as_raw_fd(), file_name)
}

pub fn apply_with_path<T: AsRef<Path>>(
    flags: u64,
    metadata: &Metadata,
    fd: RawFd,
    file_name: T,
) -> Result<(), Error> {
    apply(
        flags,
        metadata,
        fd,
        &CString::new(file_name.as_ref().as_os_str().as_bytes())?,
    )
}

pub fn apply(flags: u64, metadata: &Metadata, fd: RawFd, file_name: &CStr) -> Result<(), Error> {
    let c_proc_path = CString::new(format!("/proc/self/fd/{}", fd)).unwrap();
    let c_proc_path = c_proc_path.as_ptr();

    if metadata.stat.flags != 0 {
        todo!("apply flags!");
    }

    unsafe {
        // UID and GID first, as this fails if we lose access anyway.
        c_result!(libc::chown(
            c_proc_path,
            metadata.stat.uid,
            metadata.stat.gid
        ))
        .map(drop)
        .or_else(allow_notsupp)?;
    }

    let mut skip_xattrs = false;
    apply_xattrs(flags, c_proc_path, metadata, &mut skip_xattrs)?;
    add_fcaps(flags, c_proc_path, metadata, &mut skip_xattrs)?;
    apply_acls(flags, c_proc_path, metadata)?;
    apply_quota_project_id(flags, fd, metadata)?;

    // Finally mode and time. We may lose access with mode, but the changing the mode also
    // affects times.
    if !metadata.is_symlink() {
        c_result!(unsafe { libc::chmod(c_proc_path, perms_from_metadata(metadata)?.bits()) })
            .map(drop)
            .or_else(allow_notsupp)?;
    }

    let res = c_result!(unsafe {
        libc::utimensat(
            libc::AT_FDCWD,
            c_proc_path,
            nsec_to_update_timespec(metadata.stat.mtime).as_ptr(),
            0,
        )
    });
    match res {
        Ok(_) => (),
        Err(ref err) if err.is_errno(Errno::EOPNOTSUPP) => (),
        Err(ref err) if err.is_errno(Errno::EPERM) => {
            println!(
                "failed to restore mtime attribute on {:?}: {}",
                file_name, err
            );
        }
        Err(err) => return Err(err.into()),
    }

    Ok(())
}

fn add_fcaps(
    flags: u64,
    c_proc_path: *const libc::c_char,
    metadata: &Metadata,
    skip_xattrs: &mut bool,
) -> Result<(), Error> {
    if *skip_xattrs || !flags_contain(flags, flags::WITH_FCAPS) {
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
    .or_else(|err| allow_notsupp_remember(err, skip_xattrs))?;

    Ok(())
}

fn apply_xattrs(
    flags: u64,
    c_proc_path: *const libc::c_char,
    metadata: &Metadata,
    skip_xattrs: &mut bool,
) -> Result<(), Error> {
    if *skip_xattrs || !flags_contain(flags, flags::WITH_XATTRS) {
        return Ok(());
    }

    for xattr in &metadata.xattrs {
        if *skip_xattrs {
            return Ok(());
        }

        if !xattr::is_valid_xattr_name(xattr.name()) {
            println!("skipping invalid xattr named {:?}", xattr.name());
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
        .or_else(|err| allow_notsupp_remember(err, &mut *skip_xattrs))?;
    }

    Ok(())
}

fn apply_acls(
    flags: u64,
    c_proc_path: *const libc::c_char,
    metadata: &Metadata,
) -> Result<(), Error> {
    if !flags_contain(flags, flags::WITH_ACL) || metadata.acl.is_empty() {
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
            acl.add_entry_full(
                acl::ACL_GROUP_OBJ,
                None,
                acl::mode_group_to_acl_permissions(metadata.stat.mode),
            )?;
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

    c_try!(unsafe { acl::acl_set_file(c_proc_path, acl::ACL_TYPE_ACCESS, acl.ptr,) });
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

        c_try!(unsafe { acl::acl_set_file(c_proc_path, acl::ACL_TYPE_DEFAULT, acl.ptr,) });
    }

    Ok(())
}

fn apply_quota_project_id(flags: u64, fd: RawFd, metadata: &Metadata) -> Result<(), Error> {
    if !flags_contain(flags, flags::WITH_QUOTA_PROJID) {
        return Ok(());
    }

    let projid = match metadata.quota_project_id {
        Some(projid) => projid,
        None => return Ok(()),
    };

    let mut fsxattr = fs::FSXAttr::default();
    unsafe {
        fs::fs_ioc_fsgetxattr(fd, &mut fsxattr).map_err(|err| {
            format_err!(
                "error while getting fsxattr to restore quota project id - {}",
                err
            )
        })?;

        fsxattr.fsx_projid = projid.projid as u32;

        fs::fs_ioc_fssetxattr(fd, &fsxattr).map_err(|err| {
            format_err!(
                "error while setting fsxattr to restore quota project id - {}",
                err
            )
        })?;
    }

    Ok(())
}
