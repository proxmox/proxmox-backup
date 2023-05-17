//! Provides a very basic "newc" format cpio encoder.
//! See 'man 5 cpio' for format details, as well as:
//! <https://www.kernel.org/doc/html/latest/driver-api/early-userspace/buffer-format.html>
//! This does not provide full support for the format, only what is needed to include files in an
//! initramfs intended for a linux kernel.
use std::ffi::{CStr, CString};

use anyhow::{bail, Error};
use tokio::io::{copy, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Default)]
pub struct Entry {
    pub inode: u16,
    pub mode: u16,
    pub uid: u16,
    pub gid: u16,
    /// negative mtimes are generally valid, but cpio defines all fields as unsigned
    pub mtime: u64,
    /// c_filesize has 8 bytes, but man page claims that 4 GB files are the maximum, let's be safe
    pub size: u32,
}

/// Write a cpio file entry to an AsyncWrite.
pub async fn append_file<W: AsyncWrite + Unpin, R: AsyncRead + Unpin>(
    mut target: W,
    content: R,
    name: &CStr,
    entry: Entry,
) -> Result<(), Error> {
    let name = name.to_bytes_with_nul();

    target.write_all(b"070701").await?; // c_magic
    print_cpio_hex(&mut target, entry.inode as u64).await?; // c_ino
    print_cpio_hex(&mut target, entry.mode as u64).await?; // c_mode
    print_cpio_hex(&mut target, entry.uid as u64).await?; // c_uid
    print_cpio_hex(&mut target, entry.gid as u64).await?; // c_gid
    print_cpio_hex(&mut target, 0).await?; // c_nlink
    print_cpio_hex(&mut target, entry.mtime).await?; // c_mtime
    print_cpio_hex(&mut target, entry.size as u64).await?; // c_filesize
    print_cpio_hex(&mut target, 0).await?; // c_devmajor
    print_cpio_hex(&mut target, 0).await?; // c_devminor
    print_cpio_hex(&mut target, 0).await?; // c_rdevmajor
    print_cpio_hex(&mut target, 0).await?; // c_rdevminor
    print_cpio_hex(&mut target, name.len() as u64).await?; // c_namesize
    print_cpio_hex(&mut target, 0).await?; // c_check (ignored for newc)

    target.write_all(name).await?;
    let header_size = 6 + 8 * 13 + name.len();
    let mut name_pad = header_size;
    while name_pad & 3 != 0 {
        target.write_u8(0).await?;
        name_pad += 1;
    }

    let mut content = content.take(entry.size as u64);
    let copied = copy(&mut content, &mut target).await?;
    if copied < entry.size as u64 {
        bail!("cpio: not enough data, or size to big - encoding invalid");
    }
    let mut data_pad = copied;
    while data_pad & 3 != 0 {
        target.write_u8(0).await?;
        data_pad += 1;
    }

    Ok(())
}

/// Write the TRAILER!!! file to an AsyncWrite, signifying the end of a cpio archive. Note that you
/// can immediately add more files after, to create a concatenated archive, the kernel for example
/// will merge these upon loading an initramfs.
pub async fn append_trailer<W: AsyncWrite + Unpin>(target: W) -> Result<(), Error> {
    let name = CString::new("TRAILER!!!").unwrap();
    append_file(target, tokio::io::empty(), &name, Entry::default()).await
}

async fn print_cpio_hex<W: AsyncWrite + Unpin>(target: &mut W, value: u64) -> Result<(), Error> {
    target
        .write_all(format!("{:08x}", value).as_bytes())
        .await
        .map_err(|e| e.into())
}
