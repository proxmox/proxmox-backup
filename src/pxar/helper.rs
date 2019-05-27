use libc;
use nix::sys::stat::FileStat;

#[inline(always)]
pub fn is_directory(stat: &FileStat) -> bool {
    (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR
}

#[inline(always)]
pub fn is_symlink(stat: &FileStat) -> bool {
    (stat.st_mode & libc::S_IFMT) == libc::S_IFLNK
}

#[inline(always)]
pub fn is_reg_file(stat: &FileStat) -> bool {
    (stat.st_mode & libc::S_IFMT) == libc::S_IFREG
}

#[inline(always)]
pub fn is_block_dev(stat: &FileStat) -> bool {
    (stat.st_mode & libc::S_IFMT) == libc::S_IFBLK
}

#[inline(always)]
pub fn is_char_dev(stat: &FileStat) -> bool {
    (stat.st_mode & libc::S_IFMT) == libc::S_IFCHR
}

#[inline(always)]
pub fn is_fifo(stat: &FileStat) -> bool {
    (stat.st_mode & libc::S_IFMT) == libc::S_IFIFO
}
#[inline(always)]
pub fn is_socket(stat: &FileStat) -> bool {
    (stat.st_mode & libc::S_IFMT) == libc::S_IFSOCK
}
