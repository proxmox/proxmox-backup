//! Helpers for terminal interaction

use std::io::{Read, Write};
use std::mem::MaybeUninit;
use std::os::unix::io::AsRawFd;

use failure::*;

use proxmox::tools::try_block;

/// Returns whether the current stdin is a tty .
pub fn stdin_isatty() -> bool {
    unsafe { libc::isatty(std::io::stdin().as_raw_fd()) == 1 }
}

/// Read a password from stdin, masking the echoed output with asterisks and writing a query first.
pub fn read_password(query: &str) -> Result<Vec<u8>, Error> {
    let input = std::io::stdin();
    if unsafe { libc::isatty(input.as_raw_fd()) } != 1 {
        let mut out = String::new();
        input.read_line(&mut out)?;
        return Ok(out.into_bytes());
    }

    let mut out = std::io::stdout();
    let _ignore_error = out.write_all(query.as_bytes());
    let _ignore_error = out.flush();

    let infd = input.as_raw_fd();
    let mut termios = MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(infd, &mut *termios.as_mut_ptr()) } != 0 {
        bail!("tcgetattr() failed");
    }
    let mut termios = unsafe { termios.assume_init() };
    let old_termios = termios; // termios is a 'Copy' type
    unsafe {
        libc::cfmakeraw(&mut termios);
    }
    if unsafe { libc::tcsetattr(infd, libc::TCSANOW, &termios) } != 0 {
        bail!("tcsetattr() failed");
    }

    let mut password = Vec::<u8>::new();
    let mut asterisks = true;

    let ok: Result<(), Error> = try_block!({
        for byte in input.bytes() {
            let byte = byte?;
            match byte {
                3 => bail!("cancelled"), // ^C
                4 => break,              // ^D / EOF
                9 => asterisks = false,  // tab disables echo
                0xA | 0xD => {
                    // newline, we're done
                    let _ignore_error = out.write_all("\r\n".as_bytes());
                    let _ignore_error = out.flush();
                    break;
                }
                0x7F => {
                    // backspace
                    if !password.is_empty() {
                        password.pop();
                        if asterisks {
                            let _ignore_error = out.write_all("\x08 \x08".as_bytes());
                            let _ignore_error = out.flush();
                        }
                    }
                }
                other => {
                    password.push(other);
                    if asterisks {
                        let _ignore_error = out.write_all(b"*");
                        let _ignore_error = out.flush();
                    }
                }
            }
        }
        Ok(())
    });
    if unsafe { libc::tcsetattr(infd, libc::TCSANOW, &old_termios) } != 0 {
        // not fatal...
        eprintln!("failed to reset terminal attributes!");
    }
    match ok {
        Ok(_) => Ok(password),
        Err(e) => Err(e),
    }
}
