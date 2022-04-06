//! Helpers to work with /dev/loop* devices

use std::fs::{File, OpenOptions};
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::Path;

use anyhow::Error;

const LOOP_CONTROL: &str = "/dev/loop-control";
const LOOP_NAME: &str = "/dev/loop";

/// Implements a subset of loop device ioctls necessary to assign and release
/// a single file from a free loopdev.
mod loop_ioctl {
    use nix::{ioctl_none, ioctl_write_int_bad, ioctl_write_ptr_bad};

    const LOOP_IOCTL: u16 = 0x4C; // 'L'
    const LOOP_SET_FD: u16 = 0x00;
    const LOOP_CLR_FD: u16 = 0x01;
    const LOOP_SET_STATUS64: u16 = 0x04;

    const LOOP_CTRL_GET_FREE: u16 = 0x82;

    ioctl_write_int_bad!(ioctl_set_fd, (LOOP_IOCTL << 8) | LOOP_SET_FD);
    ioctl_none!(ioctl_clr_fd, LOOP_IOCTL, LOOP_CLR_FD);
    ioctl_none!(ioctl_ctrl_get_free, LOOP_IOCTL, LOOP_CTRL_GET_FREE);
    ioctl_write_ptr_bad!(
        ioctl_set_status64,
        (LOOP_IOCTL << 8) | LOOP_SET_STATUS64,
        LoopInfo64
    );

    pub const LO_FLAGS_READ_ONLY: u32 = 1;
    pub const LO_FLAGS_PARTSCAN: u32 = 8;

    const LO_NAME_SIZE: usize = 64;
    const LO_KEY_SIZE: usize = 32;

    #[repr(C)]
    pub struct LoopInfo64 {
        pub lo_device: u64,
        pub lo_inode: u64,
        pub lo_rdevice: u64,
        pub lo_offset: u64,
        pub lo_sizelimit: u64,
        pub lo_number: u32,
        pub lo_encrypt_type: u32,
        pub lo_encrypt_key_size: u32,
        pub lo_flags: u32,
        pub lo_file_name: [u8; LO_NAME_SIZE],
        pub lo_crypt_name: [u8; LO_NAME_SIZE],
        pub lo_encrypt_key: [u8; LO_KEY_SIZE],
        pub lo_init: [u64; 2],
    }
}

// ioctl helpers create public fns, do not export them outside the module
// users should use the wrapper functions below
use loop_ioctl::*;

/// Use the GET_FREE ioctl to get or add a free loop device, of which the
/// /dev/loopN path will be returned. This is inherently racy because of the
/// delay between this and calling assign, but since assigning is atomic it
/// does not matter much and will simply cause assign to fail.
pub fn get_or_create_free_dev() -> Result<String, Error> {
    let ctrl_file = File::open(LOOP_CONTROL)?;
    let free_num = unsafe { ioctl_ctrl_get_free(ctrl_file.as_raw_fd())? };
    let loop_file_path = format!("{}{}", LOOP_NAME, free_num);
    Ok(loop_file_path)
}

fn assign_dev(fd: RawFd, backing_fd: RawFd) -> Result<(), Error> {
    unsafe {
        ioctl_set_fd(fd, backing_fd)?;
    }

    // set required read-only flag and partscan for convenience
    let mut info: LoopInfo64 = unsafe { std::mem::zeroed() };
    info.lo_flags = LO_FLAGS_READ_ONLY | LO_FLAGS_PARTSCAN;
    unsafe {
        ioctl_set_status64(fd, &info)?;
    }

    Ok(())
}

/// Open the next available /dev/loopN file and assign the given path to
/// it as it's backing file in read-only mode.
pub fn assign<P: AsRef<Path>>(loop_dev: P, backing: P) -> Result<(), Error> {
    let loop_file = File::open(loop_dev)?;
    let backing_file = OpenOptions::new().read(true).open(backing)?;
    assign_dev(loop_file.as_raw_fd(), backing_file.as_raw_fd())?;
    Ok(())
}

/// Unassign any file descriptors currently attached to the given
/// /dev/loopN device.
pub fn unassign<P: AsRef<Path>>(path: P) -> Result<(), Error> {
    let loop_file = File::open(path)?;
    unsafe {
        ioctl_clr_fd(loop_file.as_raw_fd())?;
    }
    Ok(())
}
