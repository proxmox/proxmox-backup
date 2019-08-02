use failure::*;

use std::thread;
use std::os::unix::io::FromRawFd;
use std::path::{Path, PathBuf};
use std::io::Write;

use crate::pxar;

/// Writer implementation to deccode a .pxar archive (download).

pub struct PxarDecodeWriter {
    pipe: Option<std::fs::File>,
    child: Option<thread::JoinHandle<()>>,
}

impl Drop for PxarDecodeWriter {

    fn drop(&mut self) {
        drop(self.pipe.take());
        self.child.take().unwrap().join().unwrap();
    }
}

impl PxarDecodeWriter {

    pub fn new(base: &Path, verbose: bool) -> Result<Self, Error> {
        let (rx, tx) = nix::unistd::pipe()?;

        let base = PathBuf::from(base);
        
        let child = thread::spawn(move|| {
            let mut reader = unsafe { std::fs::File::from_raw_fd(rx) };
            let mut decoder = pxar::SequentialDecoder::new(&mut reader, pxar::flags::DEFAULT, |path| {
                if verbose {
                    println!("{:?}", path);
                }
                Ok(())
            });

            if let Err(err) = decoder.restore(&base, &Vec::new()) {
                eprintln!("pxar decode failed - {}", err);
            }
        });

        let pipe = unsafe { std::fs::File::from_raw_fd(tx) };

        Ok(Self { pipe: Some(pipe), child: Some(child) })
    }
}

impl Write for PxarDecodeWriter {

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
