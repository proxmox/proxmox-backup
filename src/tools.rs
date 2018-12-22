use failure::*;
use nix::unistd;
use nix::sys::stat;

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::io::Read;
use std::io::ErrorKind;
use std::time::Duration;

use std::os::unix::io::AsRawFd;

pub mod timer;

pub fn file_set_contents<P: AsRef<Path>>(
    path: P,
    data: &[u8],
    perm: Option<stat::Mode>,
) -> Result<(), Error> {

    let path = path.as_ref();

    // Note: we use mkstemp heŕe, because this worka with different
    // processes, threads, and even tokio tasks.
    let mut template = path.to_owned();
    template.set_extension("tmp_XXXXXX");
    let (fd, tmp_path) = match unistd::mkstemp(&template) {
        Ok((fd, path)) => (fd, path),
        Err(err) => bail!("mkstemp {:?} failed: {}", template, err),
    };

    let tmp_path = tmp_path.as_path();

    let mode : stat::Mode = perm.unwrap_or(stat::Mode::from(
        stat::Mode::S_IRUSR | stat::Mode::S_IWUSR |
        stat::Mode::S_IRGRP | stat::Mode::S_IROTH
    ));

    if let Err(err) = stat::fchmod(fd, mode) {
        let _ = unistd::unlink(tmp_path);
        bail!("fchmod {:?} failed: {}", tmp_path, err);
    }

    use std::os::unix::io::FromRawFd;
    let mut file = unsafe { File::from_raw_fd(fd) };

    if let Err(err) = file.write_all(data) {
        let _ = unistd::unlink(tmp_path);
        bail!("write failed: {}", err);
    }

    if let Err(err) = std::fs::rename(tmp_path, path) {
        let _ = unistd::unlink(tmp_path);
        bail!("Atomic rename failed for file {:?} - {}", path, err);
    }

    Ok(())
}

pub fn lock_file<F: AsRawFd>(
    file: &mut F,
    exclusive: bool,
    timeout: Option<Duration>,
    ) -> Result<(), Error>
{
    let lockarg =
        if exclusive {
            nix::fcntl::FlockArg::LockExclusive
        } else {
            nix::fcntl::FlockArg::LockShared
        };

    let timeout = match timeout {
        None => {
            nix::fcntl::flock(file.as_raw_fd(), lockarg)?;
            return Ok(());
        }
        Some(t) => t,
    };

    // unblock the timeout signal temporarily
    let _sigblock_guard = timer::unblock_timeout_signal();

    // setup a timeout timer
    let mut timer = timer::Timer::create(
        timer::Clock::Realtime,
        timer::TimerEvent::ThisThreadSignal(timer::SIGTIMEOUT))?;

    timer.arm(timer::TimerSpec::new()
        .value(Some(timeout))
        .interval(Some(Duration::from_millis(10))))?;

    nix::fcntl::flock(file.as_raw_fd(), lockarg)?;
    Ok(())
}

pub fn open_file_locked<P: AsRef<Path>>(path: P, timeout: Duration)
    -> Result<File, Error>
{
    let path = path.as_ref();
    let mut file =
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            Ok(file) => file,
            Err(err) => bail!("Unable to open lock {:?} - {}",
                              path, err),
        };
    match lock_file(&mut file, true, Some(timeout)) {
        Ok(_) => Ok(file),
        Err(err) => bail!("Unable to aquire lock {:?} - {}",
                          path, err),
    }
}

// Note: We cannot implement an Iterator, because Iterators cannot
// return a borrowed buffer ref (we want zero-copy)
pub fn file_chunker<C, R>(
    mut file: R,
    chunk_size: usize,
    mut chunk_cb: C
) -> Result<(), Error>
    where C: FnMut(usize, &[u8]) -> Result<bool, Error>,
          R: Read,
{

    const READ_BUFFER_SIZE: usize = 4*1024*1024; // 4M

    if chunk_size > READ_BUFFER_SIZE { bail!("chunk size too large!"); }

    let mut buf = vec![0u8; READ_BUFFER_SIZE];

    let mut pos = 0;
    let mut file_pos = 0;
    loop {
        let mut eof = false;
        let mut tmp = &mut buf[..];
       // try to read large portions, at least chunk_size
        while pos < chunk_size {
            match file.read(tmp) {
                Ok(0) => { eof = true; break; },
                Ok(n) => {
                    pos += n;
                    if pos > chunk_size { break; }
                    tmp = &mut tmp[n..];
                }
                Err(ref e) if e.kind() == ErrorKind::Interrupted => { /* try again */ }
                Err(e) => bail!("read chunk failed - {}", e.to_string()),
            }
        }
        let mut start = 0;
        while start + chunk_size <= pos {
            if !(chunk_cb)(file_pos, &buf[start..start+chunk_size])? { break; }
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
                unsafe { std::ptr::copy_nonoverlapping(ptr.add(start), ptr, rest); }
                pos = rest;
            } else {
                pos = 0;
            }
        }
    }

    Ok(())
}
