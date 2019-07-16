use std::io::Read;
use std::ffi::{CStr, CString};
use std::fs::File;
use std::os::unix::io::{FromRawFd, RawFd};

use failure::*;
use libc::{c_char, c_int};
use nix::fcntl::OFlag;
use nix::errno::Errno;
use nix::NixPath;
use nix::sys::stat::{FileStat, Mode};

pub const FNM_NOMATCH:  c_int = 1;

extern "C" {
    fn fnmatch(pattern: *const c_char, string: *const c_char, flags: c_int) -> c_int;
}

#[derive(Debug, PartialEq)]
pub enum MatchType {
    None,
    Exclude,
    Include,
    PartialExclude,
    PartialInclude,
}

#[derive(Clone)]
pub struct PxarExcludePattern {
    pattern: CString,
    match_exclude: bool,
    match_dir_only: bool,
    split_pattern: (CString, CString),
}

impl PxarExcludePattern {
    pub fn from_file<P: ?Sized + NixPath>(parent_fd: RawFd, filename: &P) -> Result<Option<(Vec<PxarExcludePattern>, Vec<u8>, FileStat)>, Error> {
        let stat = match nix::sys::stat::fstatat(parent_fd, filename, nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(nix::Error::Sys(Errno::ENOENT)) => return Ok(None),
            Err(err) => bail!("stat failed - {}", err),
        };

        let filefd = nix::fcntl::openat(parent_fd, filename, OFlag::O_NOFOLLOW, Mode::empty())?;
        let mut file = unsafe {
            File::from_raw_fd(filefd)
        };

        let mut content_buffer = Vec::new();
        let _bytes = file.read_to_end(&mut content_buffer)?;

        let mut exclude_pattern = Vec::new();
        for line in content_buffer.split(|&c| c == b'\n') {
            if line.is_empty() {
                continue;
            }
            if let Some(pattern) = Self::from_line(line)? {
                exclude_pattern.push(pattern);
            }
        }

        Ok(Some((exclude_pattern, content_buffer, stat)))
    }

    pub fn from_line(line: &[u8]) -> Result<Option<PxarExcludePattern>, Error> {
        let mut input = line;

        if input.starts_with(b"#") {
            return Ok(None);
        }

        let match_exclude = if input.starts_with(b"!") {
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

        Ok(Some(PxarExcludePattern {
            pattern,
            match_exclude,
            match_dir_only,
            split_pattern,
        }))
    }

    pub fn get_front_pattern(&self) -> PxarExcludePattern {
        let pattern = split_at_slash(&self.split_pattern.0);
        PxarExcludePattern {
            pattern: self.split_pattern.0.clone(),
            match_exclude: self.match_exclude,
            match_dir_only: self.match_dir_only,
            split_pattern: pattern,
        }
    }

    pub fn get_rest_pattern(&self) -> PxarExcludePattern {
        let pattern = split_at_slash(&self.split_pattern.1);
        PxarExcludePattern {
            pattern: self.split_pattern.1.clone(),
            match_exclude: self.match_exclude,
            match_dir_only: self.match_dir_only,
            split_pattern: pattern,
        }
    }

    pub fn dump(&self) {
        match (self.match_exclude, self.match_dir_only) {
            (true, true) => println!("{:#?}/", self.pattern),
            (true, false) => println!("{:#?}", self.pattern),
            (false, true) => println!("!{:#?}/", self.pattern),
            (false, false) => println!("!{:#?}", self.pattern),
        }
    }

    pub fn matches_filename(&self, filename: &CStr, is_dir: bool) -> MatchType {
        let mut res = MatchType::None;
        let (front, _) = &self.split_pattern;

        let fnmatch_res = unsafe {
            fnmatch(front.as_ptr() as *const libc::c_char, filename.as_ptr() as *const libc::c_char, 0)
        };
        // TODO error cases
        if fnmatch_res == 0 {
            res = if self.match_exclude {
                MatchType::PartialExclude
            } else {
                MatchType::PartialInclude
            };
        }

        let full = if self.pattern.to_bytes().starts_with(b"**/") {
            CString::new(&self.pattern.to_bytes()[3..]).unwrap()
        } else {
            CString::new(&self.pattern.to_bytes()[..]).unwrap()
        };
        let fnmatch_res = unsafe {
            fnmatch(full.as_ptr() as *const libc::c_char, filename.as_ptr() as *const libc::c_char, 0)
        };
        // TODO error cases
        if fnmatch_res == 0 {
            res = if self.match_exclude {
                MatchType::Exclude
            } else {
                MatchType::Include
            };
        }

        if !is_dir && self.match_dir_only {
            res = MatchType::None;
        }

        if !is_dir && (res == MatchType::PartialInclude || res == MatchType::PartialExclude) {
            res = MatchType::None;
        }

        res
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
