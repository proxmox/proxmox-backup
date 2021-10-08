//! SHA helpers.

use std::io::Read;

use anyhow::Error;

use proxmox_io::vec;

/// Calculate the sha256sum from a readable object.
pub fn sha256(file: &mut dyn Read) -> Result<([u8; 32], u64), Error> {
    let mut hasher = openssl::sha::Sha256::new();
    let mut buffer = vec::undefined(256 * 1024);
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
