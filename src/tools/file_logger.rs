use anyhow::Error;
use std::io::Write;

/// Log messages with optional automatically added timestamps into files
///
/// Logs messages to file, and optionally to standard output.
///
///
/// #### Example:
/// ```
/// #[macro_use] extern crate proxmox_backup;
/// # use anyhow::{bail, format_err, Error};
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
}

#[derive(Debug)]
pub struct FileLogger {
    file: std::fs::File,
    options: FileLogOptions,
}

/// Log messages to [FileLogger](tools/struct.FileLogger.html)
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
        let file = std::fs::OpenOptions::new()
            .read(options.read)
            .write(true)
            .append(options.append)
            .create_new(options.exclusive)
            .create(!options.exclusive)
            .open(file_name)?;

        Ok(Self { file, options })
    }

    pub fn log<S: AsRef<str>>(&mut self, msg: S) {
        let msg = msg.as_ref();

        if self.options.to_stdout {
            let mut stdout = std::io::stdout();
            stdout.write_all(msg.as_bytes()).unwrap();
            stdout.write_all(b"\n").unwrap();
        }

        let now = proxmox::tools::time::epoch_i64();
        let rfc3339 = proxmox::tools::time::epoch_to_rfc3339(now).unwrap();

        let line = if self.options.prefix_time {
            format!("{}: {}\n", rfc3339, msg)
        } else {
            format!("{}\n", msg)
        };
        self.file.write_all(line.as_bytes()).unwrap();
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
