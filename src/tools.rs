//! Tools and utilities
//!
//! This is a collection of small and useful tools.
use std::any::Any;
use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::fs::File;
use std::io::{self, BufRead, Read, Seek, SeekFrom};
use std::os::unix::io::RawFd;
use std::path::Path;

use anyhow::{bail, format_err, Error};
use serde_json::Value;
use openssl::hash::{hash, DigestBytes, MessageDigest};
use percent_encoding::{utf8_percent_encode, AsciiSet};

pub use proxmox::tools::fd::Fd;

pub mod acl;
pub mod apt;
pub mod async_io;
pub mod borrow;
pub mod cert;
pub mod daemon;
pub mod disks;
pub mod format;
pub mod fs;
pub mod fuse_loop;
pub mod http;
pub mod json;
pub mod logrotate;
pub mod loopdev;
pub mod lru_cache;
pub mod nom;
pub mod runtime;
pub mod serde_filter;
pub mod socket;
pub mod statistics;
pub mod subscription;
pub mod systemd;
pub mod ticket;
pub mod xattr;
pub mod zip;
pub mod sgutils2;
pub mod paperkey;

pub mod parallel_handler;
pub use parallel_handler::ParallelHandler;

mod wrapped_reader_stream;
pub use wrapped_reader_stream::{AsyncReaderStream, StdChannelStream, WrappedReaderStream};

mod async_channel_writer;
pub use async_channel_writer::AsyncChannelWriter;

mod std_channel_writer;
pub use std_channel_writer::StdChannelWriter;

mod tokio_writer_adapter;
pub use tokio_writer_adapter::TokioWriterAdapter;

mod process_locker;
pub use process_locker::{ProcessLocker, ProcessLockExclusiveGuard, ProcessLockSharedGuard};

mod file_logger;
pub use file_logger::{FileLogger, FileLogOptions};

mod broadcast_future;
pub use broadcast_future::{BroadcastData, BroadcastFuture};

/// The `BufferedRead` trait provides a single function
/// `buffered_read`. It returns a reference to an internal buffer. The
/// purpose of this traid is to avoid unnecessary data copies.
pub trait BufferedRead {
    /// This functions tries to fill the internal buffers, then
    /// returns a reference to the available data. It returns an empty
    /// buffer if `offset` points to the end of the file.
    fn buffered_read(&mut self, offset: u64) -> Result<&[u8], Error>;
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

pub fn required_integer_param(param: &Value, name: &str) -> Result<i64, Error> {
    match param[name].as_i64() {
        Some(s) => Ok(s),
        None => bail!("missing parameter '{}'", name),
    }
}

pub fn required_integer_property(param: &Value, name: &str) -> Result<i64, Error> {
    match param[name].as_i64() {
        Some(s) => Ok(s),
        None => bail!("missing property '{}'", name),
    }
}

pub fn required_array_param<'a>(param: &'a Value, name: &str) -> Result<&'a [Value], Error> {
    match param[name].as_array() {
        Some(s) => Ok(&s),
        None => bail!("missing parameter '{}'", name),
    }
}

pub fn required_array_property<'a>(param: &'a Value, name: &str) -> Result<&'a [Value], Error> {
    match param[name].as_array() {
        Some(s) => Ok(&s),
        None => bail!("missing property '{}'", name),
    }
}

pub fn complete_file_name<S>(arg: &str, _param: &HashMap<String, String, S>) -> Vec<String>
where
    S: BuildHasher,
{
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

    let contents = proxmox::tools::fs::file_get_contents(FILENAME)
        .map_err(|e| format_err!("Error getting host key - {}", e))?;
    let digest = md5sum(&contents)
        .map_err(|e| format_err!("Error digesting host key - {}", e))?;

    Ok(proxmox::tools::bin_to_hex(&digest).to_uppercase())
}

pub fn assert_if_modified(digest1: &str, digest2: &str) -> Result<(), Error> {
    if digest1 != digest2 {
        bail!("detected modified configuration - file changed by other user? Try again.");
    }
    Ok(())
}

/// Extract a specific cookie from cookie header.
/// We assume cookie_name is already url encoded.
pub fn extract_cookie(cookie: &str, cookie_name: &str) -> Option<String> {
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

/// percent encode a url component
pub fn percent_encode_component(comp: &str) -> String {
    utf8_percent_encode(comp, percent_encoding::NON_ALPHANUMERIC).to_string()
}

pub fn join<S: Borrow<str>>(data: &[S], sep: char) -> String {
    let mut list = String::new();

    for item in data {
        if !list.is_empty() {
            list.push(sep);
        }
        list.push_str(item.borrow());
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
) -> Result<Vec<u8>, Error> {

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

    Ok(output.stdout)
}

/// Helper to check result from std::process::Command output, returns String.
///
/// The exit_code_check() function should return true if the exit code
/// is considered successful.
pub fn command_output_as_string(
    output: std::process::Output,
    exit_code_check: Option<fn(i32) -> bool>,
) -> Result<String, Error> {
    let output = command_output(output, exit_code_check)?;
    let output = String::from_utf8(output)?;
    Ok(output)
}

pub fn run_command(
    mut command: std::process::Command,
    exit_code_check: Option<fn(i32) -> bool>,
) -> Result<String, Error> {

   let output = command.output()
        .map_err(|err| format_err!("failed to execute {:?} - {}", command, err))?;

    let output = command_output_as_string(output, exit_code_check)
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

/// Seeks to start of file and computes the SHA256 hash
pub fn compute_file_csum(file: &mut File) -> Result<([u8; 32], u64), Error> {

    file.seek(SeekFrom::Start(0))?;

    let mut hasher = openssl::sha::Sha256::new();
    let mut buffer = proxmox::tools::vec::undefined(256*1024);
    let mut size: u64 = 0;

    loop {
        let count = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(ref err) if err.kind() == std::io::ErrorKind::Interrupted => {
                continue;
            }
            Err(err) => return Err(err.into()),
        };
        size += count as u64;
        hasher.update(&buffer[..count]);
    }

    let csum = hasher.finish();

    Ok((csum, size))
}

/// Create the base run-directory.
///
/// This exists to fixate the permissions for the run *base* directory while allowing intermediate
/// directories after it to have different permissions.
pub fn create_run_dir() -> Result<(), Error> {
    let _: bool = proxmox::tools::fs::create_path(PROXMOX_BACKUP_RUN_DIR_M!(), None, None)?;
    Ok(())
}
