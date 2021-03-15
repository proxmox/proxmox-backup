//! ZIP Helper
//!
//! Provides an interface to create a ZIP File from ZipEntries
//! for a more detailed description of the ZIP format, see:
//! https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT

use std::convert::TryInto;
use std::ffi::OsString;
use std::io;
use std::mem::size_of;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use anyhow::{Error, Result};
use endian_trait::Endian;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crc32fast::Hasher;
use proxmox::tools::time::gmtime;
use proxmox::tools::byte_buffer::ByteBuffer;

const LOCAL_FH_SIG: u32 = 0x04034B50;
const LOCAL_FF_SIG: u32 = 0x08074B50;
const CENTRAL_DIRECTORY_FH_SIG: u32 = 0x02014B50;
const END_OF_CENTRAL_DIR: u32 = 0x06054B50;
const VERSION_NEEDED: u16 = 0x002d;
const VERSION_MADE_BY: u16 = 0x032d;

const ZIP64_EOCD_RECORD: u32 = 0x06064B50;
const ZIP64_EOCD_LOCATOR: u32 = 0x07064B50;

// bits for time:
// 0-4: day of the month (1-31)
// 5-8: month: (1 = jan, etc.)
// 9-15: year offset from 1980
//
// bits for date:
// 0-4: second / 2
// 5-10: minute (0-59)
// 11-15: hour (0-23)
//
// see https://docs.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-filetimetodosdatetime
fn epoch_to_dos(epoch: i64) -> (u16, u16) {
    let gmtime = match gmtime(epoch) {
        Ok(gmtime) => gmtime,
        Err(_) => return (0, 0),
    };

    let seconds = (gmtime.tm_sec / 2) & 0b11111;
    let minutes = gmtime.tm_min & 0xb111111;
    let hours = gmtime.tm_hour & 0b11111;
    let time: u16 = ((hours << 11) | (minutes << 5) | (seconds)) as u16;

    let date: u16 = if gmtime.tm_year > (2108 - 1900) || gmtime.tm_year < (1980 - 1900) {
        0
    } else {
        let day = gmtime.tm_mday & 0b11111;
        let month = (gmtime.tm_mon + 1) & 0b1111;
        let year = (gmtime.tm_year + 1900 - 1980) & 0b1111111;
        ((year << 9) | (month << 5) | (day)) as u16
    };

    (date, time)
}

#[derive(Endian)]
#[repr(C, packed)]
struct Zip64Field {
    field_type: u16,
    field_size: u16,
    uncompressed_size: u64,
    compressed_size: u64,
}

#[derive(Endian)]
#[repr(C, packed)]
struct Zip64FieldWithOffset {
    field_type: u16,
    field_size: u16,
    uncompressed_size: u64,
    compressed_size: u64,
    offset: u64,
    start_disk: u32,
}

#[derive(Endian)]
#[repr(C, packed)]
struct LocalFileHeader {
    signature: u32,
    version_needed: u16,
    flags: u16,
    compression: u16,
    time: u16,
    date: u16,
    crc32: u32,
    compressed_size: u32,
    uncompressed_size: u32,
    filename_len: u16,
    extra_field_len: u16,
}

#[derive(Endian)]
#[repr(C, packed)]
struct LocalFileFooter {
    signature: u32,
    crc32: u32,
    compressed_size: u64,
    uncompressed_size: u64,
}

#[derive(Endian)]
#[repr(C, packed)]
struct CentralDirectoryFileHeader {
    signature: u32,
    version_made_by: u16,
    version_needed: u16,
    flags: u16,
    compression: u16,
    time: u16,
    date: u16,
    crc32: u32,
    compressed_size: u32,
    uncompressed_size: u32,
    filename_len: u16,
    extra_field_len: u16,
    comment_len: u16,
    start_disk: u16,
    internal_flags: u16,
    external_flags: u32,
    offset: u32,
}

#[derive(Endian)]
#[repr(C, packed)]
struct EndOfCentralDir {
    signature: u32,
    disk_number: u16,
    start_disk: u16,
    disk_record_count: u16,
    total_record_count: u16,
    directory_size: u32,
    directory_offset: u32,
    comment_len: u16,
}

#[derive(Endian)]
#[repr(C, packed)]
struct Zip64EOCDRecord {
    signature: u32,
    field_size: u64,
    version_made_by: u16,
    version_needed: u16,
    disk_number: u32,
    disk_number_central_dir: u32,
    disk_record_count: u64,
    total_record_count: u64,
    directory_size: u64,
    directory_offset: u64,
}

#[derive(Endian)]
#[repr(C, packed)]
struct Zip64EOCDLocator {
    signature: u32,
    disk_number: u32,
    offset: u64,
    disk_count: u32,
}

async fn write_struct<E, T>(output: &mut T, data: E) -> io::Result<()>
where
    T: AsyncWrite + ?Sized + Unpin,
    E: Endian,
{
    let data = data.to_le();

    let data = unsafe {
        std::slice::from_raw_parts(
            &data as *const E as *const u8,
            core::mem::size_of_val(&data),
        )
    };
    output.write_all(data).await
}

/// Represents an Entry in a ZIP File
///
/// used to add to a ZipEncoder
pub struct ZipEntry {
    filename: OsString,
    mtime: i64,
    mode: u16,
    crc32: u32,
    uncompressed_size: u64,
    compressed_size: u64,
    offset: u64,
    is_file: bool,
}

impl ZipEntry {
    /// Creates a new ZipEntry
    ///
    /// if is_file is false the path will contain an trailing separator,
    /// so that the zip file understands that it is a directory
    pub fn new<P: AsRef<Path>>(path: P, mtime: i64, mode: u16, is_file: bool) -> Self {
        let mut relpath = PathBuf::new();

        for comp in path.as_ref().components() {
            if let Component::Normal(_) = comp {
                relpath.push(comp);
            }
        }

        if !is_file {
            relpath.push(""); // adds trailing slash
        }

        Self {
            filename: relpath.into(),
            crc32: 0,
            mtime,
            mode,
            uncompressed_size: 0,
            compressed_size: 0,
            offset: 0,
            is_file,
        }
    }

    async fn write_local_header<W>(&self, mut buf: &mut W) -> io::Result<usize>
    where
        W: AsyncWrite + Unpin + ?Sized,
    {
        let filename = self.filename.as_bytes();
        let filename_len = filename.len();
        let header_size = size_of::<LocalFileHeader>();
        let zip_field_size = size_of::<Zip64Field>();
        let size: usize = header_size + filename_len + zip_field_size;

        let (date, time) = epoch_to_dos(self.mtime);

        write_struct(
            &mut buf,
            LocalFileHeader {
                signature: LOCAL_FH_SIG,
                version_needed: 0x2d,
                flags: 1 << 3,
                compression: 0,
                time,
                date,
                crc32: 0,
                compressed_size: 0xFFFFFFFF,
                uncompressed_size: 0xFFFFFFFF,
                filename_len: filename_len as u16,
                extra_field_len: zip_field_size as u16,
            },
        )
        .await?;

        buf.write_all(filename).await?;

        write_struct(
            &mut buf,
            Zip64Field {
                field_type: 0x0001,
                field_size: 2 * 8,
                uncompressed_size: 0,
                compressed_size: 0,
            },
        )
        .await?;

        Ok(size)
    }

    async fn write_data_descriptor<W: AsyncWrite + Unpin + ?Sized>(
        &self,
        mut buf: &mut W,
    ) -> io::Result<usize> {
        let size = size_of::<LocalFileFooter>();

        write_struct(
            &mut buf,
            LocalFileFooter {
                signature: LOCAL_FF_SIG,
                crc32: self.crc32,
                compressed_size: self.compressed_size,
                uncompressed_size: self.uncompressed_size,
            },
        )
        .await?;

        Ok(size)
    }

    async fn write_central_directory_header<W: AsyncWrite + Unpin + ?Sized>(
        &self,
        mut buf: &mut W,
    ) -> io::Result<usize> {
        let filename = self.filename.as_bytes();
        let filename_len = filename.len();
        let header_size = size_of::<CentralDirectoryFileHeader>();
        let zip_field_size = size_of::<Zip64FieldWithOffset>();
        let mut size: usize = header_size + filename_len;

        let (date, time) = epoch_to_dos(self.mtime);

        let (compressed_size, uncompressed_size, offset, need_zip64) = if self.compressed_size
            >= (u32::MAX as u64)
            || self.uncompressed_size >= (u32::MAX as u64)
            || self.offset >= (u32::MAX as u64)
        {
            size += zip_field_size;
            (0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, true)
        } else {
            (
                self.compressed_size as u32,
                self.uncompressed_size as u32,
                self.offset as u32,
                false,
            )
        };

        write_struct(
            &mut buf,
            CentralDirectoryFileHeader {
                signature: CENTRAL_DIRECTORY_FH_SIG,
                version_made_by: VERSION_MADE_BY,
                version_needed: VERSION_NEEDED,
                flags: 1 << 3,
                compression: 0,
                time,
                date,
                crc32: self.crc32,
                compressed_size,
                uncompressed_size,
                filename_len: filename_len as u16,
                extra_field_len: if need_zip64 { zip_field_size as u16 } else { 0 },
                comment_len: 0,
                start_disk: 0,
                internal_flags: 0,
                external_flags: (self.mode as u32) << 16 | (!self.is_file as u32) << 4,
                offset,
            },
        )
        .await?;

        buf.write_all(filename).await?;

        if need_zip64 {
            write_struct(
                &mut buf,
                Zip64FieldWithOffset {
                    field_type: 1,
                    field_size: 3 * 8 + 4,
                    uncompressed_size: self.uncompressed_size,
                    compressed_size: self.compressed_size,
                    offset: self.offset,
                    start_disk: 0,
                },
            )
            .await?;
        }

        Ok(size)
    }
}

/// Wraps a writer that implements AsyncWrite for creating a ZIP archive
///
/// This will create a ZIP archive on the fly with files added with
/// 'add_entry'. To Finish the file, call 'finish'
/// Example:
/// ```no_run
/// use proxmox_backup::tools::zip::*;
/// use tokio::fs::File;
/// use anyhow::{Error, Result};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Error> {
///     let target = File::open("foo.zip").await?;
///     let mut source = File::open("foo.txt").await?;
///
///     let mut zip = ZipEncoder::new(target);
///     zip.add_entry(ZipEntry::new(
///         "foo.txt",
///         0,
///         0o100755,
///         true,
///     ), Some(source)).await?;
///
///     zip.finish().await?;
///
///     Ok(())
/// }
/// ```
pub struct ZipEncoder<W>
where
    W: AsyncWrite + Unpin,
{
    byte_count: usize,
    files: Vec<ZipEntry>,
    target: W,
    buf: ByteBuffer,
}

impl<W: AsyncWrite + Unpin> ZipEncoder<W> {
    pub fn new(target: W) -> Self {
        Self {
            byte_count: 0,
            files: Vec::new(),
            target,
            buf: ByteBuffer::with_capacity(1024*1024),
        }
    }

    pub async fn add_entry<R: AsyncRead + Unpin>(
        &mut self,
        mut entry: ZipEntry,
        content: Option<R>,
    ) -> Result<(), Error> {
        entry.offset = self.byte_count.try_into()?;
        self.byte_count += entry.write_local_header(&mut self.target).await?;
        if let Some(mut content) = content {
            let mut hasher = Hasher::new();
            let mut size = 0;
            loop {

                let count = self.buf.read_from_async(&mut content).await?;

                // end of file
                if count == 0 {
                    break;
                }

                size += count;
                hasher.update(&self.buf);
                self.target.write_all(&self.buf).await?;
                self.buf.consume(count);
            }

            self.byte_count += size;
            entry.compressed_size = size.try_into()?;
            entry.uncompressed_size = size.try_into()?;
            entry.crc32 = hasher.finalize();
        }
        self.byte_count += entry.write_data_descriptor(&mut self.target).await?;

        self.files.push(entry);

        Ok(())
    }

    async fn write_eocd(
        &mut self,
        central_dir_size: usize,
        central_dir_offset: usize,
    ) -> Result<(), Error> {
        let entrycount = self.files.len();

        let mut count = entrycount as u16;
        let mut directory_size = central_dir_size as u32;
        let mut directory_offset = central_dir_offset as u32;

        if central_dir_size > u32::MAX as usize
            || central_dir_offset > u32::MAX as usize
            || entrycount > u16::MAX as usize
        {
            count = 0xFFFF;
            directory_size = 0xFFFFFFFF;
            directory_offset = 0xFFFFFFFF;

            write_struct(
                &mut self.target,
                Zip64EOCDRecord {
                    signature: ZIP64_EOCD_RECORD,
                    field_size: 44,
                    version_made_by: VERSION_MADE_BY,
                    version_needed: VERSION_NEEDED,
                    disk_number: 0,
                    disk_number_central_dir: 0,
                    disk_record_count: entrycount.try_into()?,
                    total_record_count: entrycount.try_into()?,
                    directory_size: central_dir_size.try_into()?,
                    directory_offset: central_dir_offset.try_into()?,
                },
            )
            .await?;

            let locator_offset = central_dir_offset + central_dir_size;

            write_struct(
                &mut self.target,
                Zip64EOCDLocator {
                    signature: ZIP64_EOCD_LOCATOR,
                    disk_number: 0,
                    offset: locator_offset.try_into()?,
                    disk_count: 1,
                },
            )
            .await?;
        }

        write_struct(
            &mut self.target,
            EndOfCentralDir {
                signature: END_OF_CENTRAL_DIR,
                disk_number: 0,
                start_disk: 0,
                disk_record_count: count,
                total_record_count: count,
                directory_size,
                directory_offset,
                comment_len: 0,
            },
        )
        .await?;

        Ok(())
    }

    pub async fn finish(&mut self) -> Result<(), Error> {
        let central_dir_offset = self.byte_count;
        let mut central_dir_size = 0;

        for file in &self.files {
            central_dir_size += file
                .write_central_directory_header(&mut self.target)
                .await?;
        }

        self.write_eocd(central_dir_size, central_dir_offset)
            .await?;

        self.target.flush().await?;

        Ok(())
    }
}
