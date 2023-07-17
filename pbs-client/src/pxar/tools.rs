//! Some common methods used within the pxar code.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use anyhow::{bail, Context, Error};
use nix::sys::stat::Mode;

use pxar::{format::StatxTimestamp, mode, Entry, EntryKind, Metadata};

/// Get the file permissions as `nix::Mode`
pub fn perms_from_metadata(meta: &Metadata) -> Result<Mode, Error> {
    let mode = meta.stat.get_permission_bits();

    u32::try_from(mode)
        .context("couldn't narrow permission bits")
        .and_then(|mode| {
            Mode::from_bits(mode)
                .with_context(|| format!("mode contains illegal bits: 0x{:x} (0o{:o})", mode, mode))
        })
}

/// Make sure path is relative and not '.' or '..'.
pub fn assert_relative_path<S: AsRef<OsStr> + ?Sized>(path: &S) -> Result<(), Error> {
    assert_relative_path_do(Path::new(path))
}

/// Make sure path is a single component and not '.' or '..'.
pub fn assert_single_path_component<S: AsRef<OsStr> + ?Sized>(path: &S) -> Result<(), Error> {
    assert_single_path_component_do(Path::new(path))
}

fn assert_relative_path_do(path: &Path) -> Result<(), Error> {
    if !path.is_relative() {
        bail!("bad absolute file name in archive: {:?}", path);
    }

    Ok(())
}

fn assert_single_path_component_do(path: &Path) -> Result<(), Error> {
    assert_relative_path_do(path)?;

    let mut components = path.components();
    match components.next() {
        Some(std::path::Component::Normal(_)) => (),
        _ => bail!("invalid path component in archive: {:?}", path),
    }

    if components.next().is_some() {
        bail!(
            "invalid path with multiple components in archive: {:?}",
            path
        );
    }

    Ok(())
}

#[rustfmt::skip]
fn symbolic_mode(c: u64, special: bool, special_x: u8, special_no_x: u8) -> [u8; 3] {
    [
        if 0 != c & 4 { b'r' } else { b'-' },
        if 0 != c & 2 { b'w' } else { b'-' },
        match (c & 1, special) {
            (0, false) => b'-',
            (0, true) => special_no_x,
            (_, false) => b'x',
            (_, true) => special_x,
        }
    ]
}

fn mode_string(entry: &Entry) -> String {
    // https://www.gnu.org/software/coreutils/manual/html_node/What-information-is-listed.html#What-information-is-listed
    // additionally we use:
    //     file type capital 'L' hard links
    //     a second '+' after the mode to show non-acl xattr presence
    //
    // Trwxrwxrwx++ uid/gid size mtime filename [-> destination]

    let meta = entry.metadata();
    let mode = meta.stat.mode;
    let type_char = if entry.is_hardlink() {
        'L'
    } else {
        match mode & mode::IFMT {
            mode::IFREG => '-',
            mode::IFBLK => 'b',
            mode::IFCHR => 'c',
            mode::IFDIR => 'd',
            mode::IFLNK => 'l',
            mode::IFIFO => 'p',
            mode::IFSOCK => 's',
            _ => '?',
        }
    };

    let fmt_u = symbolic_mode((mode >> 6) & 7, 0 != mode & mode::ISUID, b's', b'S');
    let fmt_g = symbolic_mode((mode >> 3) & 7, 0 != mode & mode::ISGID, b's', b'S');
    let fmt_o = symbolic_mode(mode & 7, 0 != mode & mode::ISVTX, b't', b'T');

    let has_acls = if meta.acl.is_empty() { ' ' } else { '+' };

    let has_xattrs = if meta.xattrs.is_empty() { ' ' } else { '+' };

    format!(
        "{}{}{}{}{}{}",
        type_char,
        unsafe { std::str::from_utf8_unchecked(&fmt_u) },
        unsafe { std::str::from_utf8_unchecked(&fmt_g) },
        unsafe { std::str::from_utf8_unchecked(&fmt_o) },
        has_acls,
        has_xattrs,
    )
}

fn format_mtime(mtime: &StatxTimestamp) -> String {
    if let Ok(s) = proxmox_time::strftime_local("%Y-%m-%d %H:%M:%S", mtime.secs) {
        return s;
    }
    format!("{}.{}", mtime.secs, mtime.nanos)
}

pub fn format_single_line_entry(entry: &Entry) -> String {
    let mode_string = mode_string(entry);

    let meta = entry.metadata();

    let (size, link) = match entry.kind() {
        EntryKind::File { size, .. } => (format!("{}", *size), String::new()),
        EntryKind::Symlink(link) => ("0".to_string(), format!(" -> {:?}", link.as_os_str())),
        EntryKind::Hardlink(link) => ("0".to_string(), format!(" -> {:?}", link.as_os_str())),
        EntryKind::Device(dev) => (format!("{},{}", dev.major, dev.minor), String::new()),
        _ => ("0".to_string(), String::new()),
    };

    let owner_string = format!("{}/{}", meta.stat.uid, meta.stat.gid);

    format!(
        "{} {:<13} {} {:>8} {:?}{}",
        mode_string,
        owner_string,
        format_mtime(&meta.stat.mtime),
        size,
        entry.path(),
        link,
    )
}

pub fn format_multi_line_entry(entry: &Entry) -> String {
    let mode_string = mode_string(entry);

    let meta = entry.metadata();

    let (size, link, type_name) = match entry.kind() {
        EntryKind::File { size, .. } => (format!("{}", *size), String::new(), "file"),
        EntryKind::Symlink(link) => (
            "0".to_string(),
            format!(" -> {:?}", link.as_os_str()),
            "symlink",
        ),
        EntryKind::Hardlink(link) => (
            "0".to_string(),
            format!(" -> {:?}", link.as_os_str()),
            "symlink",
        ),
        EntryKind::Device(dev) => (
            format!("{},{}", dev.major, dev.minor),
            String::new(),
            if meta.stat.is_chardev() {
                "characters pecial file"
            } else if meta.stat.is_blockdev() {
                "block special file"
            } else {
                "device"
            },
        ),
        EntryKind::Socket => ("0".to_string(), String::new(), "socket"),
        EntryKind::Fifo => ("0".to_string(), String::new(), "fifo"),
        EntryKind::Directory => ("0".to_string(), String::new(), "directory"),
        EntryKind::GoodbyeTable => ("0".to_string(), String::new(), "bad entry"),
    };

    let file_name = match std::str::from_utf8(entry.path().as_os_str().as_bytes()) {
        Ok(name) => std::borrow::Cow::Borrowed(name),
        Err(_) => std::borrow::Cow::Owned(format!("{:?}", entry.path())),
    };

    format!(
        "  File: {}{}\n  \
           Size: {:<13} Type: {}\n\
         Access: ({:o}/{})  Uid: {:<5} Gid: {:<5}\n\
         Modify: {}\n",
        file_name,
        link,
        size,
        type_name,
        meta.file_mode(),
        mode_string,
        meta.stat.uid,
        meta.stat.gid,
        format_mtime(&meta.stat.mtime),
    )
}
