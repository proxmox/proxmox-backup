//! *catar* format encoder.
//!
//! This module contain the code to generate *catar* archive files.

use failure::*;
use endian_trait::Endian;

use super::format_definition::*;
use super::binary_search_tree::*;

use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};

use std::ffi::CStr;

use nix::NixPath;
use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use nix::errno::Errno;
use nix::sys::stat::FileStat;

/// The format requires to build sorted directory lookup tables in
/// memory, so we restrict the number of allowed entries to limit
/// maximum memory usage.
pub const MAX_DIRECTORY_ENTRIES: usize = 256*1024;

pub struct CaTarEncoder<'a, W: Write> {
    current_path: PathBuf, // used for error reporting
    writer: &'a mut W,
    writer_pos: usize,
    size: usize,
    file_copy_buffer: Vec<u8>,
}

impl <'a, W: Write> CaTarEncoder<'a, W> {

    pub fn encode(path: PathBuf, dir: &mut nix::dir::Dir, writer: &'a mut W) -> Result<(), Error> {

        const FILE_COPY_BUFFER_SIZE: usize = 1024*1024;

        let mut file_copy_buffer = Vec::with_capacity(FILE_COPY_BUFFER_SIZE);
        unsafe { file_copy_buffer.set_len(FILE_COPY_BUFFER_SIZE); }

        let mut me = Self {
            current_path: path,
            writer: writer,
            writer_pos: 0,
            size: 0,
            file_copy_buffer,
        };

        // todo: use scandirat??

        let dir_fd = dir.as_raw_fd();
        let stat = match nix::sys::stat::fstat(dir_fd) {
            Ok(stat) => stat,
            Err(err) => bail!("fstat {:?} failed - {}", me.current_path, err),
        };

        if (stat.st_mode & libc::S_IFMT) != libc::S_IFDIR {
            bail!("got unexpected file type {:?} (not a directory)", me.current_path);
        }

        let magic = detect_fs_type(dir_fd)?;

        if is_virtual_file_system(magic) {
            bail!("backup virtual file systems is disabled!");
        }

        me.encode_dir(dir, &stat, magic)?;

        Ok(())
    }

    fn write(&mut self,  buf: &[u8]) -> Result<(), Error> {
        self.writer.write_all(buf)?;
        self.writer_pos += buf.len();
        Ok(())
    }

    fn write_item<T: Endian>(&mut self, item: T) ->  Result<(), Error> {

        let data = item.to_le();

        let buffer = unsafe { std::slice::from_raw_parts(
            &data as *const T as *const u8,
            std::mem::size_of::<T>()
        )};

        self.write(buffer)?;

        Ok(())
    }

    fn flush_copy_buffer(&mut self, size: usize) -> Result<(), Error> {
        self.writer.write_all(&self.file_copy_buffer[..size])?;
        self.writer_pos += size;
        Ok(())
    }

    fn write_header(&mut self, htype: u64, size: u64) -> Result<(), Error> {

        let size = size + (std::mem::size_of::<CaFormatHeader>() as u64);
        self.write_item(CaFormatHeader { size, htype })?;

        Ok(())
    }

    fn write_filename(&mut self, name: &CStr) -> Result<(), Error> {

        let buffer = name.to_bytes_with_nul();
        self.write_header(CA_FORMAT_FILENAME, buffer.len() as u64)?;
        self.write(buffer)?;

        Ok(())
    }

    fn create_entry(&self, stat: &FileStat) -> Result<CaFormatEntry, Error> {

        let mode = if (stat.st_mode & libc::S_IFMT) == libc::S_IFLNK {
            (libc::S_IFLNK | 0o777) as u64
        } else {
            (stat.st_mode & (libc::S_IFMT | 0o7777)) as u64
        };

        let mtime = stat.st_mtime * 1_000_000_000 + stat.st_mtime_nsec;
        if mtime < 0 {
            bail!("got strange mtime ({}) from fstat for {:?}.", mtime, self.current_path);
        }


        let entry = CaFormatEntry {
            feature_flags: CA_FORMAT_FEATURE_FLAGS_MAX, // fixme: ??
            mode: mode,
            flags: 0,
            uid: stat.st_uid as u64,
            gid: stat.st_gid as u64,
            mtime: mtime as u64,
        };

        Ok(entry)
    }

    fn read_chattr(&self, fd: RawFd, entry: &mut CaFormatEntry) -> Result<(), Error> {

        let mut attr: usize = 0;

        let res = unsafe { read_attr_fd(fd, &mut attr)};
        if let Err(err) = res {
            if let nix::Error::Sys(errno) = err {
                if errno_is_unsupported(errno) { return Ok(()) };
            }
            bail!("read_attr_fd failed for {:?} - {}", self.current_path, err);
        }

        let flags = ca_feature_flags_from_chattr(attr as u32);
        entry.flags = entry.flags | flags;

        Ok(())
    }

    fn read_fat_attr(&self, fd: RawFd, magic: i64, entry: &mut CaFormatEntry) -> Result<(), Error> {

        if magic != MSDOS_SUPER_MAGIC && magic != FUSE_SUPER_MAGIC { return Ok(()); }

        let mut attr: u32 = 0;

        let res = unsafe { read_fat_attr_fd(fd, &mut attr)};
        if let Err(err) = res {
            if let nix::Error::Sys(errno) = err {
                if errno_is_unsupported(errno) { return Ok(()) };
            }
            bail!("read_fat_attr_fd failed for {:?} - {}", self.current_path, err);
        }

        let flags = ca_feature_flags_from_fat_attr(attr);
        entry.flags = entry.flags | flags;

        Ok(())
    }

    fn write_entry(&mut self, entry: CaFormatEntry) -> Result<(), Error> {

        self.write_header(CA_FORMAT_ENTRY, std::mem::size_of::<CaFormatEntry>() as u64)?;
        self.write_item(entry)?;

        Ok(())
    }

    fn write_goodbye_table(&mut self, goodbye_offset: usize, goodbye_items: &mut [CaFormatGoodbyeItem]) -> Result<(), Error> {

        goodbye_items.sort_unstable_by(|a, b| a.hash.cmp(&b.hash));

        let item_count = goodbye_items.len();

        let goodbye_table_size = (item_count + 1)*std::mem::size_of::<CaFormatGoodbyeItem>();

        self.write_header(CA_FORMAT_GOODBYE, goodbye_table_size as u64)?;

        if self.file_copy_buffer.len() < goodbye_table_size {
            let need = goodbye_table_size - self.file_copy_buffer.len();
            self.file_copy_buffer.reserve(need);
            unsafe { self.file_copy_buffer.set_len(self.file_copy_buffer.capacity()); }
        }

        let buffer = &mut self.file_copy_buffer;

        copy_binary_search_tree(item_count, |s, d| {
            let item = &goodbye_items[s];
            let offset = d*std::mem::size_of::<CaFormatGoodbyeItem>();
            let dest = crate::tools::map_struct_mut::<CaFormatGoodbyeItem>(&mut buffer[offset..]).unwrap();
            dest.offset = u64::to_le(item.offset);
            dest.size = u64::to_le(item.size);
            dest.hash = u64::to_le(item.hash);
        });

        // append CaFormatGoodbyeTail as last item
        let offset = item_count*std::mem::size_of::<CaFormatGoodbyeItem>();
        let dest = crate::tools::map_struct_mut::<CaFormatGoodbyeItem>(&mut buffer[offset..]).unwrap();
        dest.offset = u64::to_le(goodbye_offset as u64);
        dest.size = u64::to_le((goodbye_table_size + std::mem::size_of::<CaFormatHeader>()) as u64);
        dest.hash = u64::to_le(CA_FORMAT_GOODBYE_TAIL_MARKER);

        self.flush_copy_buffer(goodbye_table_size)?;

        Ok(())
    }

    fn encode_dir(&mut self, dir: &mut nix::dir::Dir, dir_stat: &FileStat, magic: i64)  -> Result<(), Error> {

        //println!("encode_dir: {:?} start {}", self.current_path, self.writer_pos);

        let mut name_list = vec![];

        let rawfd = dir.as_raw_fd();

        let dir_start_pos = self.writer_pos;

        let mut dir_entry = self.create_entry(&dir_stat)?;

        self.read_chattr(rawfd, &mut dir_entry)?;
        self.read_fat_attr(rawfd, magic, &mut dir_entry)?;

        self.write_entry(dir_entry)?;

        let mut dir_count = 0;

        if !is_virtual_file_system(magic) {
            for entry in dir.iter() {
                dir_count += 1;
                if dir_count > MAX_DIRECTORY_ENTRIES {
                    bail!("too many directory items in {:?} (> {})",
                          self.current_path, MAX_DIRECTORY_ENTRIES);
                }

                let entry = match entry {
                    Ok(entry) => entry,
                    Err(err) => bail!("readir {:?} failed - {}", self.current_path, err),
                };
                let filename = entry.file_name().to_owned();

                let name = filename.to_bytes_with_nul();
                let name_len = name.len();
                if name_len == 2 && name[0] == b'.' && name[1] == 0u8 { continue; }
                if name_len == 3 && name[0] == b'.' && name[1] == b'.' && name[2] == 0u8 { continue; }

                name_list.push(filename);
            }
        }

        name_list.sort_unstable_by(|a, b| a.cmp(&b));

        let mut goodbye_items = vec![];

        for filename in &name_list {
            self.current_path.push(std::ffi::OsStr::from_bytes(filename.as_bytes()));

            let stat = match nix::sys::stat::fstatat(rawfd, filename.as_ref(), nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW) {
                Ok(stat) => stat,
                Err(nix::Error::Sys(Errno::ENOENT)) => {
                    self.report_vanished_file(&self.current_path)?;
                    continue;
                }
                Err(err) => bail!("fstat {:?} failed - {}", self.current_path, err),
            };

            let start_pos = self.writer_pos;

            let ifmt = stat.st_mode & libc::S_IFMT;

            if ifmt == libc::S_IFDIR {

                let mut dir = match nix::dir::Dir::openat(rawfd, filename.as_ref(), OFlag::O_DIRECTORY|OFlag::O_NOFOLLOW, Mode::empty()) {
                    Ok(dir) => dir,
                    Err(nix::Error::Sys(Errno::ENOENT)) => {
                        self.report_vanished_file(&self.current_path)?;
                        continue; // fixme!!
                    },
                    Err(err) => bail!("open dir {:?} failed - {}", self.current_path, err),
                };

                let child_magic = if dir_stat.st_dev != stat.st_dev {
                    detect_fs_type(dir.as_raw_fd())?
                } else {
                    magic
                };

                self.write_filename(&filename)?;
                self.encode_dir(&mut dir, &stat, child_magic)?;

            } else if ifmt == libc::S_IFREG {
                let filefd = match nix::fcntl::openat(rawfd, filename.as_ref(), OFlag::O_NOFOLLOW, Mode::empty()) {
                    Ok(filefd) => filefd,
                    Err(nix::Error::Sys(Errno::ENOENT)) => {
                        self.report_vanished_file(&self.current_path)?;
                        continue;
                    },
                    Err(err) => bail!("open file {:?} failed - {}", self.current_path, err),
                };

                let child_magic = if dir_stat.st_dev != stat.st_dev {
                    detect_fs_type(filefd)?
                } else {
                    magic
                };

                self.write_filename(&filename)?;
                let res = self.encode_file(filefd, &stat, child_magic);
                let _ = nix::unistd::close(filefd); // ignore close errors
                res?;

            } else if ifmt == libc::S_IFLNK {
                let mut buffer = [0u8; libc::PATH_MAX as usize];

                let res = filename.with_nix_path(|cstr| {
                    unsafe { libc::readlinkat(rawfd, cstr.as_ptr(), buffer.as_mut_ptr() as *mut libc::c_char, buffer.len()-1) }
                })?;

                match Errno::result(res) {
                    Ok(len) => {
                        buffer[len as usize] = 0u8; // add Nul byte
                        self.write_filename(&filename)?;
                        self.encode_symlink(&buffer[..((len+1) as usize)], &stat)?
                    }
                    Err(nix::Error::Sys(Errno::ENOENT)) => {
                        self.report_vanished_file(&self.current_path)?;
                        continue;
                    }
                    Err(err) => bail!("readlink {:?} failed - {}", self.current_path, err),
                }
            } else if (ifmt == libc::S_IFBLK) || (ifmt == libc::S_IFCHR) {
                self.write_filename(&filename)?;
                self.encode_device(&stat)?;
            } else if (ifmt == libc::S_IFIFO) || (ifmt == libc::S_IFSOCK) {
                self.write_filename(&filename)?;
                self.encode_special(&stat)?;
            } else {
                bail!("unsupported file type (mode {:o} {:?})", stat.st_mode, self.current_path);
            }

            let end_pos = self.writer_pos;

            goodbye_items.push(CaFormatGoodbyeItem {
                offset: start_pos as u64,
                size: (end_pos - start_pos) as u64,
                hash: compute_goodbye_hash(filename.to_bytes()),
            });

            self.current_path.pop();
        }

        //println!("encode_dir: {:?} end {}", self.current_path, self.writer_pos);

        // fixup goodby item offsets
        let goodbye_start = self.writer_pos as u64;
        for item in &mut goodbye_items {
            item.offset = goodbye_start - item.offset;
        }

        let goodbye_offset = self.writer_pos - dir_start_pos;

        self.write_goodbye_table(goodbye_offset, &mut goodbye_items)?;

        //println!("encode_dir: {:?} end1 {}", self.current_path, self.writer_pos);
        Ok(())
    }

    fn encode_file(&mut self, filefd: RawFd, stat: &FileStat, magic: i64)  -> Result<(), Error> {

        //println!("encode_file: {:?}", self.current_path);

        let mut entry = self.create_entry(&stat)?;

        self.read_chattr(filefd, &mut entry)?;
        self.read_fat_attr(filefd, magic, &mut entry)?;

        self.write_entry(entry)?;

        if is_virtual_file_system(magic) {
            self.write_header(CA_FORMAT_PAYLOAD, 0)?;
            return Ok(());
        }

        let size = stat.st_size as u64;

        self.write_header(CA_FORMAT_PAYLOAD, size)?;

        let mut pos: u64 = 0;
        loop {
            let n = match nix::unistd::read(filefd, &mut self.file_copy_buffer) {
                Ok(n) => n,
                Err(nix::Error::Sys(Errno::EINTR)) => continue /* try again */,
                Err(err) =>  bail!("read {:?} failed - {}", self.current_path, err),
            };
            if n == 0 { // EOF
                if pos != size {
                    // Note:: casync format cannot handle that
                    bail!("detected shrinked file {:?} ({} < {})", self.current_path, pos, size);
                }
                break;
            }

            let mut next = pos + (n as u64);

            if next > size { next = size; }

            let count = (next - pos) as usize;

            self.flush_copy_buffer(count)?;

            pos = next;

            if pos >= size { break; }
        }

        Ok(())
    }

    fn encode_device(&mut self, stat: &FileStat)  -> Result<(), Error> {

        let entry = self.create_entry(&stat)?;

        self.write_entry(entry)?;

        let major = unsafe { libc::major(stat.st_rdev) } as u64;
        let minor = unsafe { libc::minor(stat.st_rdev) } as u64;

        println!("encode_device: {:?} {} {} {}", self.current_path, stat.st_rdev, major, minor);

        self.write_header(CA_FORMAT_DEVICE, std::mem::size_of::<CaFormatDevice>() as u64)?;
        self.write_item(CaFormatDevice { major, minor })?;

        Ok(())
    }

    // FIFO or Socket
    fn encode_special(&mut self, stat: &FileStat)  -> Result<(), Error> {

        let entry = self.create_entry(&stat)?;

        self.write_entry(entry)?;

        Ok(())
    }

    fn encode_symlink(&mut self, target: &[u8], stat: &FileStat)  -> Result<(), Error> {

        //println!("encode_symlink: {:?} -> {:?}", self.current_path, target);

        let entry = self.create_entry(&stat)?;
        self.write_entry(entry)?;

        self.write_header(CA_FORMAT_SYMLINK, target.len() as u64)?;
        self.write(target)?;

        Ok(())
    }

    // the report_XXX method may raise and error - depending on encoder configuration

    fn report_vanished_file(&self, path: &Path) -> Result<(), Error> {

        eprintln!("WARNING: detected vanished file {:?}", path);

        Ok(())
    }
}

fn errno_is_unsupported(errno: Errno) -> bool {

    match errno {
        Errno::ENOTTY | Errno::ENOSYS | Errno::EBADF | Errno::EOPNOTSUPP | Errno::EINVAL => {
            true
        }
        _ => false,
    }
}

fn detect_fs_type(fd: RawFd) -> Result<i64, Error> {
    let mut fs_stat: libc::statfs = unsafe { std::mem::uninitialized() };
    let res = unsafe { libc::fstatfs(fd, &mut fs_stat) };
    Errno::result(res)?;

    Ok(fs_stat.f_type)
}

use nix::{convert_ioctl_res, request_code_read, ioc};

// /usr/include/linux/fs.h: #define FS_IOC_GETFLAGS _IOR('f', 1, long)
/// read Linux file system attributes (see man chattr)
nix::ioctl_read!(read_attr_fd, b'f', 1, usize);

// /usr/include/linux/msdos_fs.h: #define FAT_IOCTL_GET_ATTRIBUTES _IOR('r', 0x10, __u32)
// read FAT file system attributes
nix::ioctl_read!(read_fat_attr_fd, b'r', 0x10, u32);


// from /usr/include/linux/magic.h
// and from casync util.h
const BINFMTFS_MAGIC: i64 =        0x42494e4d;
const CGROUP2_SUPER_MAGIC: i64 =   0x63677270;
const CGROUP_SUPER_MAGIC: i64 =    0x0027e0eb;
const CONFIGFS_MAGIC: i64 =        0x62656570;
const DEBUGFS_MAGIC: i64 =         0x64626720;
const DEVPTS_SUPER_MAGIC: i64 =    0x00001cd1;
const EFIVARFS_MAGIC: i64 =        0xde5e81e4;
const FUSE_CTL_SUPER_MAGIC: i64 =  0x65735543;
const HUGETLBFS_MAGIC: i64 =       0x958458f6;
const MQUEUE_MAGIC: i64 =          0x19800202;
const NFSD_MAGIC: i64 =            0x6e667364;
const PROC_SUPER_MAGIC: i64 =      0x00009fa0;
const PSTOREFS_MAGIC: i64 =        0x6165676C;
const RPCAUTH_GSSMAGIC: i64 =      0x67596969;
const SECURITYFS_MAGIC: i64 =      0x73636673;
const SELINUX_MAGIC: i64 =         0xf97cff8c;
const SMACK_MAGIC: i64 =           0x43415d53;
const RAMFS_MAGIC: i64 =           0x858458f6;
const TMPFS_MAGIC: i64 =           0x01021994;
const SYSFS_MAGIC: i64 =           0x62656572;
const MSDOS_SUPER_MAGIC: i64 =     0x00004d44;
const FUSE_SUPER_MAGIC: i64 =      0x65735546;


#[inline(always)]
fn is_temporary_file_system(magic: i64) -> bool {
    magic == RAMFS_MAGIC || magic == TMPFS_MAGIC
}

fn is_virtual_file_system(magic: i64) -> bool {

    match magic {
        BINFMTFS_MAGIC |
        CGROUP2_SUPER_MAGIC |
        CGROUP_SUPER_MAGIC |
        CONFIGFS_MAGIC |
        DEBUGFS_MAGIC |
        DEVPTS_SUPER_MAGIC |
        EFIVARFS_MAGIC |
        FUSE_CTL_SUPER_MAGIC |
        HUGETLBFS_MAGIC |
        MQUEUE_MAGIC |
        NFSD_MAGIC |
        PROC_SUPER_MAGIC |
        PSTOREFS_MAGIC |
        RPCAUTH_GSSMAGIC |
        SECURITYFS_MAGIC |
        SELINUX_MAGIC |
        SMACK_MAGIC |
        SYSFS_MAGIC => true,
        _ => false
    }
}
