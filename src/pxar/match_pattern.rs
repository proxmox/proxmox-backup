use std::io::Read;
use std::ffi::{CStr, CString};
use std::fs::File;
use std::os::unix::io::{FromRawFd, RawFd};

use failure::*;
use libc::{c_char, c_int};
use nix::fcntl;
use nix::fcntl::{AtFlags, OFlag};
use nix::errno::Errno;
use nix::NixPath;
use nix::sys::stat;
use nix::sys::stat::{FileStat, Mode};

pub const FNM_NOMATCH: c_int = 1;

extern "C" {
    fn fnmatch(pattern: *const c_char, string: *const c_char, flags: c_int) -> c_int;
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum MatchType {
    None,
    Positive,
    Negative,
    PartialPositive,
    PartialNegative,
}

#[derive(Clone)]
pub struct MatchPattern {
    pattern: CString,
    match_positive: bool,
    match_dir_only: bool,
    split_pattern: (CString, CString),
}

impl MatchPattern {
    pub fn from_file<P: ?Sized + NixPath>(
        parent_fd: RawFd,
        filename: &P,
    ) -> Result<Option<(Vec<MatchPattern>, Vec<u8>, FileStat)>, Error> {

        let stat = match stat::fstatat(parent_fd, filename, AtFlags::AT_SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(nix::Error::Sys(Errno::ENOENT)) => return Ok(None),
            Err(err) => bail!("stat failed - {}", err),
        };

        let filefd = fcntl::openat(parent_fd, filename, OFlag::O_NOFOLLOW, Mode::empty())?;
        let mut file = unsafe {
            File::from_raw_fd(filefd)
        };

        let mut content_buffer = Vec::new();
        let _bytes = file.read_to_end(&mut content_buffer)?;

        let mut match_pattern = Vec::new();
        for line in content_buffer.split(|&c| c == b'\n') {
            if line.is_empty() {
                continue;
            }
            if let Some(pattern) = Self::from_line(line)? {
                match_pattern.push(pattern);
            }
        }

        Ok(Some((match_pattern, content_buffer, stat)))
    }

    pub fn from_line(line: &[u8]) -> Result<Option<MatchPattern>, Error> {
        let mut input = line;

        if input.starts_with(b"#") {
            return Ok(None);
        }

        let match_positive = if input.starts_with(b"!") {
            // Reduce slice view to exclude "!"
            input = &input[1..];
            false
        } else {
            true
        };

        // Paths ending in / match only directory names (no filenames)
        let match_dir_only = if input.ends_with(b"/") {
            let len = input.len();
            input = &input[..len - 1];
            true
        } else {
            false
        };

        // Ignore initial slash
        if input.starts_with(b"/") {
            input = &input[1..];
        }

        if input.is_empty() || input == b"." ||
            input == b".." || input.contains(&b'\0') {
            bail!("invalid path component encountered");
        }

        // This will fail if the line contains b"\0"
        let pattern = CString::new(input)?;
        let split_pattern = split_at_slash(&pattern);

        Ok(Some(MatchPattern {
            pattern,
            match_positive,
            match_dir_only,
            split_pattern,
        }))
    }

    pub fn get_front_pattern(&self) -> MatchPattern {
        let pattern = split_at_slash(&self.split_pattern.0);
        MatchPattern {
            pattern: self.split_pattern.0.clone(),
            match_positive: self.match_positive,
            match_dir_only: self.match_dir_only,
            split_pattern: pattern,
        }
    }

    pub fn get_rest_pattern(&self) -> MatchPattern {
        let pattern = split_at_slash(&self.split_pattern.1);
        MatchPattern {
            pattern: self.split_pattern.1.clone(),
            match_positive: self.match_positive,
            match_dir_only: self.match_dir_only,
            split_pattern: pattern,
        }
    }

    pub fn dump(&self) {
        match (self.match_positive, self.match_dir_only) {
            (true, true) => println!("{:#?}/", self.pattern),
            (true, false) => println!("{:#?}", self.pattern),
            (false, true) => println!("!{:#?}/", self.pattern),
            (false, false) => println!("!{:#?}", self.pattern),
        }
    }

    pub fn matches_filename(&self, filename: &CStr, is_dir: bool) -> Result<MatchType, Error> {
        let mut res = MatchType::None;
        let (front, _) = &self.split_pattern;

        let fnmatch_res = unsafe {
            let front_ptr = front.as_ptr() as *const libc::c_char;
            let filename_ptr = filename.as_ptr() as *const libc::c_char;
            fnmatch(front_ptr, filename_ptr , 0)
        };
        if fnmatch_res < 0 {
            bail!("error in fnmatch inside of MatchPattern");
        }
        if fnmatch_res == 0 {
            res = if self.match_positive {
                MatchType::PartialPositive
            } else {
                MatchType::PartialNegative
            };
        }

        let full = if self.pattern.to_bytes().starts_with(b"**/") {
            CString::new(&self.pattern.to_bytes()[3..]).unwrap()
        } else {
            CString::new(&self.pattern.to_bytes()[..]).unwrap()
        };
        let fnmatch_res = unsafe {
            let full_ptr = full.as_ptr() as *const libc::c_char;
            let filename_ptr = filename.as_ptr() as *const libc::c_char;
            fnmatch(full_ptr, filename_ptr, 0)
        };
        if fnmatch_res < 0 {
            bail!("error in fnmatch inside of MatchPattern");
        }
        if fnmatch_res == 0 {
            res = if self.match_positive {
                MatchType::Positive
            } else {
                MatchType::Negative
            };
        }

        if !is_dir && self.match_dir_only {
            res = MatchType::None;
        }

        if !is_dir && (res == MatchType::PartialPositive || res == MatchType::PartialNegative) {
            res = MatchType::None;
        }

        Ok(res)
    }
}

fn split_at_slash(match_pattern: &CStr) -> (CString, CString) {
    let match_pattern = match_pattern.to_bytes();

    let pattern = if match_pattern.starts_with(b"./") {
        &match_pattern[2..]
    } else {
        match_pattern
    };

    let (mut front, mut rest) = match pattern.iter().position(|&c| c == b'/') {
        Some(ind) => {
            let (front, rest) = pattern.split_at(ind);
            (front, &rest[1..])
        },
        None => (pattern, &pattern[0..0]),
    };
    // '**' is treated such that it maches any directory
    if front == b"**" {
        front = b"*";
        rest = pattern;
    }

    // Pattern where valid CStrings before, so it is safe to unwrap the Result
    let front_pattern = CString::new(front).unwrap();
    let rest_pattern = CString::new(rest).unwrap();
    (front_pattern, rest_pattern)
}
