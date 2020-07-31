//! Tools and utilities
//!
//! This is a collection of small and useful tools.
use std::any::Any;
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, ErrorKind, Read};
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::Path;
use std::time::Duration;
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};

use anyhow::{bail, format_err, Error};
use serde_json::Value;
use openssl::hash::{hash, DigestBytes, MessageDigest};
use percent_encoding::AsciiSet;

use proxmox::tools::vec;
use proxmox::sys::error::SysResult;

pub use proxmox::tools::fd::Fd;

pub mod acl;
pub mod async_io;
pub mod borrow;
pub mod cert;
pub mod daemon;
pub mod disks;
pub mod fs;
pub mod format;
pub mod lru_cache;
pub mod runtime;
pub mod ticket;
pub mod timer;
pub mod statistics;
pub mod systemd;
pub mod nom;

mod wrapped_reader_stream;
pub use wrapped_reader_stream::*;

mod std_channel_writer;
pub use std_channel_writer::*;

pub mod xattr;

mod process_locker;
pub use process_locker::*;

mod file_logger;
pub use file_logger::*;

mod broadcast_future;
pub use broadcast_future::*;

/// The `BufferedRead` trait provides a single function
/// `buffered_read`. It returns a reference to an internal buffer. The
/// purpose of this traid is to avoid unnecessary data copies.
pub trait BufferedRead {
    /// This functions tries to fill the internal buffers, then
    /// returns a reference to the available data. It returns an empty
    /// buffer if `offset` points to the end of the file.
    fn buffered_read(&mut self, offset: u64) -> Result<&[u8], Error>;
}

/// Directly map a type into a binary buffer. This is mostly useful
/// for reading structured data from a byte stream (file). You need to
/// make sure that the buffer location does not change, so please
/// avoid vec resize while you use such map.
///
/// This function panics if the buffer is not large enough.
pub fn map_struct<T>(buffer: &[u8]) -> Result<&T, Error> {
    if buffer.len() < ::std::mem::size_of::<T>() {
        bail!("unable to map struct - buffer too small");
    }
    Ok(unsafe { &*(buffer.as_ptr() as *const T) })
}

/// Directly map a type into a mutable binary buffer. This is mostly
/// useful for writing structured data into a byte stream (file). You
/// need to make sure that the buffer location does not change, so
/// please avoid vec resize while you use such map.
///
/// This function panics if the buffer is not large enough.
pub fn map_struct_mut<T>(buffer: &mut [u8]) -> Result<&mut T, Error> {
    if buffer.len() < ::std::mem::size_of::<T>() {
        bail!("unable to map struct - buffer too small");
    }
    Ok(unsafe { &mut *(buffer.as_ptr() as *mut T) })
}

/// Create a file lock using fntl. This function allows you to specify
/// a timeout if you want to avoid infinite blocking.
///
/// With timeout set to 0, non-blocking mode is used and the function
/// will fail immediately if the lock can't be acquired.
pub fn lock_file<F: AsRawFd>(
    file: &mut F,
    exclusive: bool,
    timeout: Option<Duration>,
) -> Result<(), io::Error> {
    let lockarg = if exclusive {
        nix::fcntl::FlockArg::LockExclusive
    } else {
        nix::fcntl::FlockArg::LockShared
    };

    let timeout = match timeout {
        None => {
            nix::fcntl::flock(file.as_raw_fd(), lockarg).into_io_result()?;
            return Ok(());
        }
        Some(t) => t,
    };

    if timeout.as_nanos() == 0 {
        let lockarg = if exclusive {
            nix::fcntl::FlockArg::LockExclusiveNonblock
        } else {
            nix::fcntl::FlockArg::LockSharedNonblock
        };
        nix::fcntl::flock(file.as_raw_fd(), lockarg).into_io_result()?;
        return Ok(());
    }

    // unblock the timeout signal temporarily
    let _sigblock_guard = timer::unblock_timeout_signal();

    // setup a timeout timer
    let mut timer = timer::Timer::create(
        timer::Clock::Realtime,
        timer::TimerEvent::ThisThreadSignal(timer::SIGTIMEOUT),
    )?;

    timer.arm(
        timer::TimerSpec::new()
            .value(Some(timeout))
            .interval(Some(Duration::from_millis(10))),
    )?;

    nix::fcntl::flock(file.as_raw_fd(), lockarg).into_io_result()?;
    Ok(())
}

/// Open or create a lock file (append mode). Then try to
/// acquire a lock using `lock_file()`.
pub fn open_file_locked<P: AsRef<Path>>(path: P, timeout: Duration) -> Result<File, Error> {
    let path = path.as_ref();
    let mut file = match OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => file,
        Err(err) => bail!("Unable to open lock {:?} - {}", path, err),
    };
    match lock_file(&mut file, true, Some(timeout)) {
        Ok(_) => Ok(file),
        Err(err) => bail!("Unable to acquire lock {:?} - {}", path, err),
    }
}

/// Split a file into equal sized chunks. The last chunk may be
/// smaller. Note: We cannot implement an `Iterator`, because iterators
/// cannot return a borrowed buffer ref (we want zero-copy)
pub fn file_chunker<C, R>(mut file: R, chunk_size: usize, mut chunk_cb: C) -> Result<(), Error>
where
    C: FnMut(usize, &[u8]) -> Result<bool, Error>,
    R: Read,
{
    const READ_BUFFER_SIZE: usize = 4 * 1024 * 1024; // 4M

    if chunk_size > READ_BUFFER_SIZE {
        bail!("chunk size too large!");
    }

    let mut buf = vec::undefined(READ_BUFFER_SIZE);

    let mut pos = 0;
    let mut file_pos = 0;
    loop {
        let mut eof = false;
        let mut tmp = &mut buf[..];
        // try to read large portions, at least chunk_size
        while pos < chunk_size {
            match file.read(tmp) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(n) => {
                    pos += n;
                    if pos > chunk_size {
                        break;
                    }
                    tmp = &mut tmp[n..];
                }
                Err(ref e) if e.kind() == ErrorKind::Interrupted => { /* try again */ }
                Err(e) => bail!("read chunk failed - {}", e.to_string()),
            }
        }
        let mut start = 0;
        while start + chunk_size <= pos {
            if !(chunk_cb)(file_pos, &buf[start..start + chunk_size])? {
                break;
            }
            file_pos += chunk_size;
            start += chunk_size;
        }
        if eof {
            if start < pos {
                (chunk_cb)(file_pos, &buf[start..pos])?;
                //file_pos += pos - start;
            }
            break;
        } else {
            let rest = pos - start;
            if rest > 0 {
                let ptr = buf.as_mut_ptr();
                unsafe {
                    std::ptr::copy_nonoverlapping(ptr.add(start), ptr, rest);
                }
                pos = rest;
            } else {
                pos = 0;
            }
        }
    }

    Ok(())
}

pub fn json_object_to_query(data: Value) -> Result<String, Error> {
    let mut query = url::form_urlencoded::Serializer::new(String::new());

    let object = data.as_object().ok_or_else(|| {
        format_err!("json_object_to_query: got wrong data type (expected object).")
    })?;

    for (key, value) in object {
        match value {
            Value::Bool(b) => {
                query.append_pair(key, &b.to_string());
            }
            Value::Number(n) => {
                query.append_pair(key, &n.to_string());
            }
            Value::String(s) => {
                query.append_pair(key, &s);
            }
            Value::Array(arr) => {
                for element in arr {
                    match element {
                        Value::Bool(b) => {
                            query.append_pair(key, &b.to_string());
                        }
                        Value::Number(n) => {
                            query.append_pair(key, &n.to_string());
                        }
                        Value::String(s) => {
                            query.append_pair(key, &s);
                        }
                        _ => bail!(
                            "json_object_to_query: unable to handle complex array data types."
                        ),
                    }
                }
            }
            _ => bail!("json_object_to_query: unable to handle complex data types."),
        }
    }

    Ok(query.finish())
}

pub fn required_string_param<'a>(param: &'a Value, name: &str) -> Result<&'a str, Error> {
    match param[name].as_str() {
        Some(s) => Ok(s),
        None => bail!("missing parameter '{}'", name),
    }
}

pub fn required_string_property<'a>(param: &'a Value, name: &str) -> Result<&'a str, Error> {
    match param[name].as_str() {
        Some(s) => Ok(s),
        None => bail!("missing property '{}'", name),
    }
}

pub fn required_integer_param<'a>(param: &'a Value, name: &str) -> Result<i64, Error> {
    match param[name].as_i64() {
        Some(s) => Ok(s),
        None => bail!("missing parameter '{}'", name),
    }
}

pub fn required_integer_property<'a>(param: &'a Value, name: &str) -> Result<i64, Error> {
    match param[name].as_i64() {
        Some(s) => Ok(s),
        None => bail!("missing property '{}'", name),
    }
}

pub fn required_array_param<'a>(param: &'a Value, name: &str) -> Result<Vec<Value>, Error> {
    match param[name].as_array() {
        Some(s) => Ok(s.to_vec()),
        None => bail!("missing parameter '{}'", name),
    }
}

pub fn required_array_property<'a>(param: &'a Value, name: &str) -> Result<Vec<Value>, Error> {
    match param[name].as_array() {
        Some(s) => Ok(s.to_vec()),
        None => bail!("missing property '{}'", name),
    }
}

pub fn complete_file_name<S: BuildHasher>(arg: &str, _param: &HashMap<String, String, S>) -> Vec<String> {
    let mut result = vec![];

    use nix::fcntl::AtFlags;
    use nix::fcntl::OFlag;
    use nix::sys::stat::Mode;

    let mut dirname = std::path::PathBuf::from(if arg.is_empty() { "./" } else { arg });

    let is_dir = match nix::sys::stat::fstatat(libc::AT_FDCWD, &dirname, AtFlags::empty()) {
        Ok(stat) => (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR,
        Err(_) => false,
    };

    if !is_dir {
        if let Some(parent) = dirname.parent() {
            dirname = parent.to_owned();
        }
    }

    let mut dir =
        match nix::dir::Dir::openat(libc::AT_FDCWD, &dirname, OFlag::O_DIRECTORY, Mode::empty()) {
            Ok(d) => d,
            Err(_) => return result,
        };

    for item in dir.iter() {
        if let Ok(entry) = item {
            if let Ok(name) = entry.file_name().to_str() {
                if name == "." || name == ".." {
                    continue;
                }
                let mut newpath = dirname.clone();
                newpath.push(name);

                if let Ok(stat) =
                    nix::sys::stat::fstatat(libc::AT_FDCWD, &newpath, AtFlags::empty())
                {
                    if (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR {
                        newpath.push("");
                        if let Some(newpath) = newpath.to_str() {
                            result.push(newpath.to_owned());
                        }
                        continue;
                    }
                }
                if let Some(newpath) = newpath.to_str() {
                    result.push(newpath.to_owned());
                }
            }
        }
    }

    result
}

/// Scan directory for matching file names.
///
/// Scan through all directory entries and call `callback()` function
/// if the entry name matches the regular expression. This function
/// used unix `openat()`, so you can pass absolute or relative file
/// names. This function simply skips non-UTF8 encoded names.
pub fn scandir<P, F>(
    dirfd: RawFd,
    path: &P,
    regex: &regex::Regex,
    mut callback: F,
) -> Result<(), Error>
where
    F: FnMut(RawFd, &str, nix::dir::Type) -> Result<(), Error>,
    P: ?Sized + nix::NixPath,
{
    for entry in self::fs::scan_subdir(dirfd, path, regex)? {
        let entry = entry?;
        let file_type = match entry.file_type() {
            Some(file_type) => file_type,
            None => bail!("unable to detect file type"),
        };

        callback(
            entry.parent_fd(),
            unsafe { entry.file_name_utf8_unchecked() },
            file_type,
        )?;
    }
    Ok(())
}

/// Shortcut for md5 sums.
pub fn md5sum(data: &[u8]) -> Result<DigestBytes, Error> {
    hash(MessageDigest::md5(), data).map_err(Error::from)
}

pub fn get_hardware_address() -> Result<String, Error> {
    static FILENAME: &str = "/etc/ssh/ssh_host_rsa_key.pub";

    let contents = proxmox::tools::fs::file_get_contents(FILENAME)?;
    let digest = md5sum(&contents)?;

    Ok(proxmox::tools::bin_to_hex(&digest))
}

pub fn assert_if_modified(digest1: &str, digest2: &str) -> Result<(), Error> {
    if digest1 != digest2 {
        bail!("detected modified configuration - file changed by other user? Try again.");
    }
    Ok(())
}

/// Extract authentication cookie from cookie header.
/// We assume cookie_name is already url encoded.
pub fn extract_auth_cookie(cookie: &str, cookie_name: &str) -> Option<String> {
    for pair in cookie.split(';') {
        let (name, value) = match pair.find('=') {
            Some(i) => (pair[..i].trim(), pair[(i + 1)..].trim()),
            None => return None, // Cookie format error
        };

        if name == cookie_name {
            use percent_encoding::percent_decode;
            if let Ok(value) = percent_decode(value.as_bytes()).decode_utf8() {
                return Some(value.into());
            } else {
                return None; // Cookie format error
            }
        }
    }

    None
}

pub fn join(data: &Vec<String>, sep: char) -> String {
    let mut list = String::new();

    for item in data {
        if !list.is_empty() {
            list.push(sep);
        }
        list.push_str(item);
    }

    list
}

/// Detect modified configuration files
///
/// This function fails with a reasonable error message if checksums do not match.
pub fn detect_modified_configuration_file(digest1: &[u8;32], digest2: &[u8;32]) -> Result<(), Error> {
    if digest1 != digest2 {
	bail!("detected modified configuration - file changed by other user? Try again.");
    }
    Ok(())
}

/// normalize uri path
///
/// Do not allow ".", "..", or hidden files ".XXXX"
/// Also remove empty path components
pub fn normalize_uri_path(path: &str) -> Result<(String, Vec<&str>), Error> {
    let items = path.split('/');

    let mut path = String::new();
    let mut components = vec![];

    for name in items {
        if name.is_empty() {
            continue;
        }
        if name.starts_with('.') {
            bail!("Path contains illegal components.");
        }
        path.push('/');
        path.push_str(name);
        components.push(name);
    }

    Ok((path, components))
}

/// Helper to check result from std::process::Command output
///
/// The exit_code_check() function should return true if the exit code
/// is considered successful.
pub fn command_output(
    output: std::process::Output,
    exit_code_check: Option<fn(i32) -> bool>,
) -> Result<String, Error> {

    if !output.status.success() {
        match output.status.code() {
            Some(code) => {
                let is_ok = match exit_code_check {
                    Some(check_fn) => check_fn(code),
                    None => code == 0,
                };
                if !is_ok {
                    let msg = String::from_utf8(output.stderr)
                        .map(|m| if m.is_empty() { String::from("no error message") } else { m })
                        .unwrap_or_else(|_| String::from("non utf8 error message (suppressed)"));

                    bail!("status code: {} - {}", code, msg);
                }
            }
            None => bail!("terminated by signal"),
        }
    }

    let output = String::from_utf8(output.stdout)?;

    Ok(output)
}

pub fn run_command(
    mut command: std::process::Command,
    exit_code_check: Option<fn(i32) -> bool>,
) -> Result<String, Error> {

   let output = command.output()
        .map_err(|err| format_err!("failed to execute {:?} - {}", command, err))?;

    let output = crate::tools::command_output(output, exit_code_check)
        .map_err(|err| format_err!("command {:?} failed - {}", command, err))?;

    Ok(output)
}

pub fn fd_change_cloexec(fd: RawFd, on: bool) -> Result<(), Error> {
    use nix::fcntl::{fcntl, FdFlag, F_GETFD, F_SETFD};
    let mut flags = FdFlag::from_bits(fcntl(fd, F_GETFD)?)
        .ok_or_else(|| format_err!("unhandled file flags"))?; // nix crate is stupid this way...
    flags.set(FdFlag::FD_CLOEXEC, on);
    fcntl(fd, F_SETFD(flags))?;
    Ok(())
}

static mut SHUTDOWN_REQUESTED: bool = false;

pub fn request_shutdown() {
    unsafe {
        SHUTDOWN_REQUESTED = true;
    }
    crate::server::server_shutdown();
}

#[inline(always)]
pub fn shutdown_requested() -> bool {
    unsafe { SHUTDOWN_REQUESTED }
}

pub fn fail_on_shutdown() -> Result<(), Error> {
    if shutdown_requested() {
        bail!("Server shutdown requested - aborting task");
    }
    Ok(())
}

/// safe wrapper for `nix::unistd::pipe2` defaulting to `O_CLOEXEC` and guarding the file
/// descriptors.
pub fn pipe() -> Result<(Fd, Fd), Error> {
    let (pin, pout) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)?;
    Ok((Fd(pin), Fd(pout)))
}

/// safe wrapper for `nix::sys::socket::socketpair` defaulting to `O_CLOEXEC` and guarding the file
/// descriptors.
pub fn socketpair() -> Result<(Fd, Fd), Error> {
    use nix::sys::socket;
    let (pa, pb) = socket::socketpair(
        socket::AddressFamily::Unix,
        socket::SockType::Stream,
        None,
        socket::SockFlag::SOCK_CLOEXEC,
    )?;
    Ok((Fd(pa), Fd(pb)))
}


/// An easy way to convert types to Any
///
/// Mostly useful to downcast trait objects (see RpcEnvironment).
pub trait AsAny {
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// This used to be: `SIMPLE_ENCODE_SET` plus space, `"`, `#`, `<`, `>`, backtick, `?`, `{`, `}`
pub const DEFAULT_ENCODE_SET: &AsciiSet = &percent_encoding::CONTROLS // 0..1f and 7e
    // The SIMPLE_ENCODE_SET adds space and anything >= 0x7e (7e itself is already included above)
    .add(0x20)
    .add(0x7f)
    // the DEFAULT_ENCODE_SET added:
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'?')
    .add(b'{')
    .add(b'}');

/// Get an iterator over lines of a file, skipping empty lines and comments (lines starting with a
/// `#`).
pub fn file_get_non_comment_lines<P: AsRef<Path>>(
    path: P,
) -> Result<impl Iterator<Item = io::Result<String>>, Error> {
    let path = path.as_ref();

    Ok(io::BufReader::new(
        File::open(path).map_err(|err| format_err!("error opening {:?}: {}", path, err))?,
    )
    .lines()
    .filter_map(|line| match line {
        Ok(line) => {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                None
            } else {
                Some(Ok(line.to_string()))
            }
        }
        Err(err) => Some(Err(err)),
    }))
}

pub fn epoch_now() -> Result<Duration, SystemTimeError> {
    SystemTime::now().duration_since(UNIX_EPOCH)
}

pub fn epoch_now_f64() -> Result<f64, SystemTimeError> {
    Ok(epoch_now()?.as_secs_f64())
}

pub fn epoch_now_u64() -> Result<u64, SystemTimeError> {
    Ok(epoch_now()?.as_secs())
}

pub fn setup_safe_path_env() {
    std::env::set_var("PATH", "/sbin:/bin:/usr/sbin:/usr/bin");
    // Make %ENV safer - as suggested by https://perldoc.perl.org/perlsec.html
    for name in &["IFS", "CDPATH", "ENV", "BASH_ENV"] {
        std::env::remove_var(name);
    }
}

pub fn strip_ascii_whitespace(line: &[u8]) -> &[u8] {
    let line = match line.iter().position(|&b| !b.is_ascii_whitespace()) {
        Some(n) => &line[n..],
        None => return &[],
    };
    match line.iter().rev().position(|&b| !b.is_ascii_whitespace()) {
        Some(n) => &line[..(line.len() - n)],
        None => &[],
    }
}
