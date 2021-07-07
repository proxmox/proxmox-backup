use anyhow::Error;
use std::io::Write;

/// Log messages with optional automatically added timestamps into files
///
/// Logs messages to file, and optionally to standard output.
///
///
/// #### Example:
/// ```
/// # use anyhow::{bail, format_err, Error};
/// use proxmox_backup::flog;
/// use proxmox_backup::tools::{FileLogger, FileLogOptions};
///
/// # std::fs::remove_file("test.log");
/// let options = FileLogOptions {
///     to_stdout: true,
///     exclusive: true,
///     ..Default::default()
/// };
/// let mut log = FileLogger::new("test.log", options).unwrap();
/// flog!(log, "A simple log: {}", "Hello!");
/// # std::fs::remove_file("test.log");
/// ```

#[derive(Debug, Default)]
/// Options to control the behavior of a ['FileLogger'] instance
pub struct FileLogOptions {
    /// Open underlying log file in append mode, useful when multiple concurrent processes
    /// want to log to the same file (e.g., HTTP access log). Note that it is only atomic
    /// for writes smaller than the PIPE_BUF (4k on Linux).
    /// Inside the same process you may need to still use an mutex, for shared access.
    pub append: bool,
    /// Open underlying log file as readable
    pub read: bool,
    /// If set, ensure that the file is newly created or error out if already existing.
    pub exclusive: bool,
    /// Duplicate logged messages to STDOUT, like tee
    pub to_stdout: bool,
    /// Prefix messages logged to the file with the current local time as RFC 3339
    pub prefix_time: bool,
    /// if set, the file is tried to be chowned by the backup:backup user/group
    /// Note, this is not designed race free as anybody could set it to another user afterwards
    /// anyway. It must thus be used by all processes which doe not run as backup uid/gid.
    pub owned_by_backup: bool,
}

#[derive(Debug)]
pub struct FileLogger {
    file: std::fs::File,
    file_name: std::path::PathBuf,
    options: FileLogOptions,
}

/// Log messages to [`FileLogger`](tools/struct.FileLogger.html)
#[macro_export]
macro_rules! flog {
    ($log:expr, $($arg:tt)*) => ({
        $log.log(format!($($arg)*));
    })
}

impl FileLogger {
    pub fn new<P: AsRef<std::path::Path>>(
        file_name: P,
        options: FileLogOptions,
    ) -> Result<Self, Error> {
        let file = Self::open(&file_name, &options)?;

        let file_name: std::path::PathBuf = file_name.as_ref().to_path_buf();

        Ok(Self { file, file_name, options })
    }

    pub fn reopen(&mut self) -> Result<&Self, Error> {
        let file = Self::open(&self.file_name, &self.options)?;
        self.file = file;
        Ok(self)
    }

    fn open<P: AsRef<std::path::Path>>(
        file_name: P,
        options: &FileLogOptions,
    ) -> Result<std::fs::File, Error> {
        let file = std::fs::OpenOptions::new()
            .read(options.read)
            .write(true)
            .append(options.append)
            .create_new(options.exclusive)
            .create(!options.exclusive)
            .open(&file_name)?;

        if options.owned_by_backup {
            let backup_user = crate::backup::backup_user()?;
            nix::unistd::chown(file_name.as_ref(), Some(backup_user.uid), Some(backup_user.gid))?;
        }

        Ok(file)
    }

    pub fn log<S: AsRef<str>>(&mut self, msg: S) {
        let msg = msg.as_ref();

        if self.options.to_stdout {
            let mut stdout = std::io::stdout();
            stdout.write_all(msg.as_bytes()).unwrap();
            stdout.write_all(b"\n").unwrap();
        }

        let line = if self.options.prefix_time {
            let now = proxmox::tools::time::epoch_i64();
            let rfc3339 = match proxmox::tools::time::epoch_to_rfc3339(now) {
                Ok(rfc3339) => rfc3339,
                Err(_) => "1970-01-01T00:00:00Z".into(), // for safety, should really not happen!
            };
            format!("{}: {}\n", rfc3339, msg)
        } else {
            format!("{}\n", msg)
        };
        if let Err(err) = self.file.write_all(line.as_bytes()) {
            // avoid panicking, log methods should not do that
            // FIXME: or, return result???
            eprintln!("error writing to log file - {}", err);
        }
    }
}

impl std::io::Write for FileLogger {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        if self.options.to_stdout {
            let _ = std::io::stdout().write(buf);
        }
        self.file.write(buf)
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        if self.options.to_stdout {
            let _ = std::io::stdout().flush();
        }
        self.file.flush()
    }
}
