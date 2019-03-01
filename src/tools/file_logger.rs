use failure::*;
use chrono::Local;
use std::io::Write;

/// Log messages with timestamps into files
///
/// Logs messages to file, and optionaly to standart output.
///
///
/// #### Example:
/// ```
/// #[macro_use] extern crate proxmox_backup;
/// # use failure::*;
/// use proxmox_backup::tools::FileLogger;
///
/// let mut log = FileLogger::new("test.log", true).unwrap();
/// flog!(log, "A simple log: {}", "Hello!");
/// ```


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

    pub fn new(file_name: &str, to_stdout: bool) -> Result<Self, Error> {

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
            stdout.write(msg.as_bytes()).unwrap();
            stdout.write(b"\n").unwrap();
        }


        let line = format!("{}: {}\n", Local::now().format("%b %e %T"), msg);
        self.file.write(line.as_bytes()).unwrap();
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
