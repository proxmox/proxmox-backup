use failure::*;

use std::thread;
use std::os::unix::io::FromRawFd;
use std::path::{Path, PathBuf};
use std::io::Write;
use std::ffi::OsString;

//use nix::fcntl::OFlag;
//use nix::sys::stat::Mode;
//use nix::dir::Dir;

use crate::catar::decoder::*;

/// Writer implementation to deccode a .catar archive (download).

pub struct CaTarBackupWriter {
    pipe: Option<std::fs::File>,
    child: Option<thread::JoinHandle<()>>,
}

impl Drop for CaTarBackupWriter {

    fn drop(&mut self) {
        drop(self.pipe.take());
        self.child.take().unwrap().join().unwrap();
    }
}

impl CaTarBackupWriter {

    pub fn new(base: &Path, subdir: OsString, verbose: bool) -> Result<Self, Error> {
        let (rx, tx) = nix::unistd::pipe()?;

        let dir = match nix::dir::Dir::open(base, nix::fcntl::OFlag::O_DIRECTORY,  nix::sys::stat::Mode::empty()) {
            Ok(dir) => dir,
            Err(err) => bail!("unable to open target directory {:?} - {}", base, err),
        };
        let mut path = PathBuf::from(base);
        path.push(&subdir);
        
        let child = thread::spawn(move|| {
            let mut reader = unsafe { std::fs::File::from_raw_fd(rx) };
            let mut decoder = CaTarDecoder::new(&mut reader);

            
            if let Err(err) = decoder.restore_sequential(&mut path, &subdir, &dir, false, & |path| {
                println!("RESTORE: {:?}", path);
                Ok(())
            }) {
                eprintln!("catar decode failed - {}", err);
            }
        });

        let pipe = unsafe { std::fs::File::from_raw_fd(tx) };

        Ok(Self { pipe: Some(pipe), child: Some(child) })
    }
}

impl Write for CaTarBackupWriter {

    fn write(&mut self, buffer: &[u8]) -> Result<usize, std::io::Error> {
        let pipe = match self.pipe {
            Some(ref mut pipe) => pipe,
            None => unreachable!(),
        };
        pipe.write(buffer)
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        let pipe = match self.pipe {
            Some(ref mut pipe) => pipe,
            None => unreachable!(),
        };
        pipe.flush()
    }
}
