use failure::*;
use nix::unistd;
use nix::sys::stat;

use std::fs::File;
use std::io::Write;
use std::path::Path;

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
