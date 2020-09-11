use anyhow::{Error};
use chrono::Local;
use std::io::Write;

/// Log messages with timestamps into files
///
/// Logs messages to file, and optionally to standard output.
///
///
/// #### Example:
/// ```
/// #[macro_use] extern crate proxmox_backup;
/// # use anyhow::{bail, format_err, Error};
/// use proxmox_backup::tools::FileLogger;
///
/// # std::fs::remove_file("test.log");
/// let mut log = FileLogger::new("test.log", true).unwrap();
/// flog!(log, "A simple log: {}", "Hello!");
/// ```


#[derive(Debug)]
pub struct FileLogger {
    file: std::fs::File,
    to_stdout: bool,
}

/// Log messages to [FileLogger](tools/struct.FileLogger.html)
#[macro_export]
macro_rules! flog {
    ($log:expr, $($arg:tt)*) => ({
        $log.log(format!($($arg)*));
    })
}

impl FileLogger {

    pub fn new<P: AsRef<std::path::Path>>(file_name: P, to_stdout: bool) -> Result<Self, Error> {

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(file_name)?;

        Ok(Self { file , to_stdout })
    }

    pub fn log<S: AsRef<str>>(&mut self, msg: S) {

        let msg = msg.as_ref();

        let mut stdout = std::io::stdout();
        if self.to_stdout {
            stdout.write_all(msg.as_bytes()).unwrap();
            stdout.write_all(b"\n").unwrap();
        }

        let line = format!("{}: {}\n", Local::now().to_rfc3339(), msg);
        self.file.write_all(line.as_bytes()).unwrap();
    }
}

impl std::io::Write for FileLogger {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        if self.to_stdout { let _ = std::io::stdout().write(buf); }
        self.file.write(buf)
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        if self.to_stdout { let _ = std::io::stdout().flush(); }
        self.file.flush()
    }
}
