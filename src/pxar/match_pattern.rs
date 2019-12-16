//! `MatchPattern` defines a match pattern used to match filenames encountered
//! during encoding or decoding of a `pxar` archive.
//! `fnmatch` is used internally to match filenames against the patterns.
//! Shell wildcard pattern can be used to match multiple filenames, see manpage
//! `glob(7)`.
//! `**` is treated special, as it matches multiple directories in a path.

use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::Read;
use std::os::unix::io::{FromRawFd, RawFd};

use failure::{bail, Error};
use libc::{c_char, c_int};
use nix::errno::Errno;
use nix::fcntl;
use nix::fcntl::{AtFlags, OFlag};
use nix::sys::stat;
use nix::sys::stat::{FileStat, Mode};
use nix::NixPath;

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

/// `MatchPattern` provides functionality for filename glob pattern matching
/// based on glibc's `fnmatch`.
/// Positive matches return `MatchType::PartialPositive` or `MatchType::Positive`.
/// Patterns starting with `!` are interpreted as negation, meaning they will
/// return `MatchType::PartialNegative` or `MatchType::Negative`.
/// No matches result in `MatchType::None`.
/// # Examples:
/// ```
/// # use std::ffi::CString;
/// # use self::proxmox_backup::pxar::{MatchPattern, MatchType};
/// # fn main() -> Result<(), failure::Error> {
/// let filename = CString::new("some.conf")?;
/// let is_dir = false;
///
/// /// Positive match of any file ending in `.conf` in any subdirectory
/// let positive = MatchPattern::from_line(b"**/*.conf")?.unwrap();
/// let m_positive = positive.as_slice().matches_filename(&filename, is_dir)?;
/// assert!(m_positive == MatchType::Positive);
///
/// /// Negative match of filenames starting with `s`
/// let negative = MatchPattern::from_line(b"![s]*")?.unwrap();
/// let m_negative = negative.as_slice().matches_filename(&filename, is_dir)?;
/// assert!(m_negative == MatchType::Negative);
/// # Ok(())
/// # }
/// ```
#[derive(Eq, PartialOrd)]
pub struct MatchPattern {
    pattern: Vec<u8>,
    match_positive: bool,
    match_dir_only: bool,
}

impl std::cmp::PartialEq for MatchPattern {
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern
        && self.match_positive == other.match_positive
        && self.match_dir_only == other.match_dir_only
    }
}

impl std::cmp::Ord for MatchPattern {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (&self.pattern, &self.match_positive, &self.match_dir_only)
            .cmp(&(&other.pattern, &other.match_positive, &other.match_dir_only))
    }
}

impl MatchPattern {
    /// Read a list of `MatchPattern` from file.
    /// The file is read line by line (lines terminated by newline character),
    /// each line may only contain one pattern.
    /// Leading `/` are ignored and lines starting with `#` are interpreted as
    /// comments and not included in the resulting list.
    /// Patterns ending in `/` will match only directories.
    ///
    /// On success, a list of match pattern is returned as well as the raw file
    /// byte buffer together with the files stats.
    /// This is done in order to avoid reading the file more than once during
    /// encoding of the archive.
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
        let mut file = unsafe { File::from_raw_fd(filefd) };

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

    /// Interprete a byte buffer as a sinlge line containing a valid
    /// `MatchPattern`.
    /// Pattern starting with `#` are interpreted as comments, returning `Ok(None)`.
    /// Pattern starting with '!' are interpreted as negative match pattern.
    /// Pattern with trailing `/` match only against directories.
    /// `.` as well as `..` and any pattern containing `\0` are invalid and will
    /// result in an error.
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

        if input.is_empty() || input == b"." || input == b".." || input.contains(&b'\0') {
            bail!("invalid path component encountered");
        }

        Ok(Some(MatchPattern {
            pattern: input.to_vec(),
            match_positive,
            match_dir_only,
        }))
    }


    /// Create a `MatchPatternSlice` of the `MatchPattern` to give a view of the
    /// `MatchPattern` without copying its content.
    pub fn as_slice<'a>(&'a self) -> MatchPatternSlice<'a> {
        MatchPatternSlice {
            pattern: self.pattern.as_slice(),
            match_positive: self.match_positive,
            match_dir_only: self.match_dir_only,
        }
    }

    /// Dump the content of the `MatchPattern` to stdout.
    /// Intended for debugging purposes only.
    pub fn dump(&self) {
        match (self.match_positive, self.match_dir_only) {
            (true, true) => println!("{:#?}/", self.pattern),
            (true, false) => println!("{:#?}", self.pattern),
            (false, true) => println!("!{:#?}/", self.pattern),
            (false, false) => println!("!{:#?}", self.pattern),
        }
    }

    /// Convert a list of MatchPattern to bytes in order to write them to e.g.
    /// a file.
    pub fn to_bytes(patterns: &[MatchPattern]) -> Vec<u8> {
        let mut slices = Vec::new();
        for pattern in patterns {
            slices.push(pattern.as_slice());
        }

        MatchPatternSlice::to_bytes(&slices)
    }

    /// Invert the match type for this MatchPattern.
    pub fn invert(&mut self) {
        self.match_positive = !self.match_positive;
    }
}

#[derive(Clone)]
pub struct MatchPatternSlice<'a> {
    pattern: &'a [u8],
    match_positive: bool,
    match_dir_only: bool,
}

impl<'a> MatchPatternSlice<'a> {
    /// Returns the pattern before the first `/` encountered as `MatchPatternSlice`.
    /// If no slash is encountered, the `MatchPatternSlice` will be a copy of the
    /// original pattern.
    /// ```
    /// # use self::proxmox_backup::pxar::{MatchPattern, MatchPatternSlice, MatchType};
    /// # fn main() -> Result<(), failure::Error> {
    /// let pattern = MatchPattern::from_line(b"some/match/pattern/")?.unwrap();
    /// let slice = pattern.as_slice();
    /// let front = slice.get_front_pattern();
    /// /// ... will be the same as ...
    /// let front_pattern = MatchPattern::from_line(b"some")?.unwrap();
    /// let front_slice = front_pattern.as_slice();
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_front_pattern(&'a self) -> MatchPatternSlice<'a> {
        let (front, _) = self.split_at_slash();
        MatchPatternSlice {
            pattern: front,
            match_positive: self.match_positive,
            match_dir_only: self.match_dir_only,
        }
    }

    /// Returns the pattern after the first encountered `/` as `MatchPatternSlice`.
    /// If no slash is encountered, the `MatchPatternSlice` will be empty.
    /// ```
    /// # use self::proxmox_backup::pxar::{MatchPattern, MatchPatternSlice, MatchType};
    /// # fn main() -> Result<(), failure::Error> {
    /// let pattern = MatchPattern::from_line(b"some/match/pattern/")?.unwrap();
    /// let slice = pattern.as_slice();
    /// let rest = slice.get_rest_pattern();
    /// /// ... will be the same as ...
    /// let rest_pattern = MatchPattern::from_line(b"match/pattern/")?.unwrap();
    /// let rest_slice = rest_pattern.as_slice();
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_rest_pattern(&'a self) -> MatchPatternSlice<'a> {
        let (_, rest) = self.split_at_slash();
        MatchPatternSlice {
            pattern: rest,
            match_positive: self.match_positive,
            match_dir_only: self.match_dir_only,
        }
    }

    /// Splits the `MatchPatternSlice` at the first slash encountered and returns the
    /// content before (front pattern) and after the slash (rest pattern),
    /// omitting the slash itself.
    /// Slices starting with `**/` are an exception to this, as the corresponding
    /// `MatchPattern` is intended to match multiple directories.
    /// These pattern slices therefore return a `*` as front pattern and the original
    /// pattern itself as rest pattern.
    fn split_at_slash(&'a self) -> (&'a [u8], &'a [u8]) {
        let pattern = if self.pattern.starts_with(b"./") {
            &self.pattern[2..]
        } else {
            self.pattern
        };

        let (mut front, mut rest) = match pattern.iter().position(|&c| c == b'/') {
            Some(ind) => {
                let (front, rest) = pattern.split_at(ind);
                (front, &rest[1..])
            }
            None => (pattern, &pattern[0..0]),
        };
        // '**' is treated such that it maches any directory
        if front == b"**" {
            front = b"*";
            rest = pattern;
        }

        (front, rest)
    }

    /// Convert a list of `MatchPatternSlice`s to bytes in order to write them to e.g.
    /// a file.
    pub fn to_bytes(patterns: &[MatchPatternSlice]) -> Vec<u8> {
        let mut buffer = Vec::new();
        for pattern in patterns {
            if !pattern.match_positive { buffer.push(b'!'); }
            buffer.extend_from_slice(&pattern.pattern);
            if pattern.match_dir_only { buffer.push(b'/'); }
            buffer.push(b'\n');
        }
        buffer
    }

    /// Match the given filename against this `MatchPatternSlice`.
    /// If the filename matches the pattern completely, `MatchType::Positive` or
    /// `MatchType::Negative` is returned, depending if the match pattern is was
    /// declared as positive (no `!` prefix) or negative (`!` prefix).
    /// If the pattern matched only up to the first slash of the pattern,
    /// `MatchType::PartialPositive` or `MatchType::PartialNegatie` is returned.
    /// If the pattern was postfixed by a trailing `/` a match is only valid if
    /// the parameter `is_dir` equals `true`.
    /// No match results in `MatchType::None`.
    pub fn matches_filename(&self, filename: &CStr, is_dir: bool) -> Result<MatchType, Error> {
        let mut res = MatchType::None;
        let (front, _) = self.split_at_slash();

        let front = CString::new(front).unwrap();
        let fnmatch_res = unsafe {
            let front_ptr = front.as_ptr() as *const libc::c_char;
            let filename_ptr = filename.as_ptr() as *const libc::c_char;
            fnmatch(front_ptr, filename_ptr, 0)
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

        let full = if self.pattern.starts_with(b"**/") {
            CString::new(&self.pattern[3..]).unwrap()
        } else {
            CString::new(&self.pattern[..]).unwrap()
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

    /// Match the given filename against the set of `MatchPatternSlice`s.
    ///
    /// A positive match is intended to includes the full subtree (unless another
    /// negative match excludes entries later).
    /// The `MatchType` together with an updated `MatchPatternSlice` list for passing
    /// to the matched child is returned.
    /// ```
    /// # use std::ffi::CString;
    /// # use self::proxmox_backup::pxar::{MatchPattern, MatchPatternSlice, MatchType};
    /// # fn main() -> Result<(), failure::Error> {
    /// let patterns = vec![
    ///     MatchPattern::from_line(b"some/match/pattern/")?.unwrap(),
    ///     MatchPattern::from_line(b"to_match/")?.unwrap()
    /// ];
    /// let mut slices = Vec::new();
    /// for pattern in &patterns {
    ///     slices.push(pattern.as_slice());
    /// }
    /// let filename = CString::new("some")?;
    /// let is_dir = true;
    /// let (match_type, child_pattern) = MatchPatternSlice::match_filename_include(
    ///     &filename,
    ///     is_dir,
    ///     &slices
    /// )?;
    /// assert_eq!(match_type, MatchType::PartialPositive);
    /// /// child pattern will be the same as ...
    /// let pattern = MatchPattern::from_line(b"match/pattern/")?.unwrap();
    /// let slice = pattern.as_slice();
    ///
    /// let filename = CString::new("to_match")?;
    /// let is_dir = true;
    /// let (match_type, child_pattern) = MatchPatternSlice::match_filename_include(
    ///     &filename,
    ///     is_dir,
    ///     &slices
    /// )?;
    /// assert_eq!(match_type, MatchType::Positive);
    /// /// child pattern will be the same as ...
    /// let pattern = MatchPattern::from_line(b"**/*")?.unwrap();
    /// let slice = pattern.as_slice();
    /// # Ok(())
    /// # }
    /// ```
    pub fn match_filename_include(
        filename: &CStr,
        is_dir: bool,
        match_pattern: &'a [MatchPatternSlice<'a>],
    ) -> Result<(MatchType, Vec<MatchPatternSlice<'a>>), Error> {
        let mut child_pattern = Vec::new();
        let mut match_state = MatchType::None;

        for pattern in match_pattern {
            match pattern.matches_filename(filename, is_dir)? {
                MatchType::None => continue,
                MatchType::Positive => match_state = MatchType::Positive,
                MatchType::Negative => match_state = MatchType::Negative,
                MatchType::PartialPositive => {
                    if match_state != MatchType::Negative && match_state != MatchType::Positive {
                        match_state = MatchType::PartialPositive;
                    }
                    child_pattern.push(pattern.get_rest_pattern());
                }
                MatchType::PartialNegative => {
                    if match_state == MatchType::PartialPositive {
                        match_state = MatchType::PartialNegative;
                    }
                    child_pattern.push(pattern.get_rest_pattern());
                }
            }
        }

        Ok((match_state, child_pattern))
    }

    /// Match the given filename against the set of `MatchPatternSlice`s.
    ///
    /// A positive match is intended to exclude the full subtree, independent of
    /// matches deeper down the tree.
    /// The `MatchType` together with an updated `MatchPattern` list for passing
    /// to the matched child is returned.
    /// ```
    /// # use std::ffi::CString;
    /// # use self::proxmox_backup::pxar::{MatchPattern, MatchPatternSlice, MatchType};
    /// # fn main() -> Result<(), failure::Error> {
    /// let patterns = vec![
    ///     MatchPattern::from_line(b"some/match/pattern/")?.unwrap(),
    ///     MatchPattern::from_line(b"to_match/")?.unwrap()
    /// ];
    /// let mut slices = Vec::new();
    /// for pattern in &patterns {
    ///     slices.push(pattern.as_slice());
    /// }
    /// let filename = CString::new("some")?;
    /// let is_dir = true;
    /// let (match_type, child_pattern) = MatchPatternSlice::match_filename_exclude(
    ///     &filename,
    ///     is_dir,
    ///     &slices,
    /// )?;
    /// assert_eq!(match_type, MatchType::PartialPositive);
    /// /// child pattern will be the same as ...
    /// let pattern = MatchPattern::from_line(b"match/pattern/")?.unwrap();
    /// let slice = pattern.as_slice();
    ///
    /// let filename = CString::new("to_match")?;
    /// let is_dir = true;
    /// let (match_type, child_pattern) = MatchPatternSlice::match_filename_exclude(
    ///     &filename,
    ///     is_dir,
    ///     &slices,
    /// )?;
    /// assert_eq!(match_type, MatchType::Positive);
    /// /// child pattern will be empty
    /// # Ok(())
    /// # }
    /// ```
    pub fn match_filename_exclude(
        filename: &CStr,
        is_dir: bool,
        match_pattern: &'a [MatchPatternSlice<'a>],
    ) -> Result<(MatchType, Vec<MatchPatternSlice<'a>>), Error> {
        let mut child_pattern = Vec::new();
        let mut match_state = MatchType::None;

        for pattern in match_pattern {
            match pattern.matches_filename(filename, is_dir)? {
                MatchType::None => {}
                MatchType::Positive => match_state = MatchType::Positive,
                MatchType::Negative => match_state = MatchType::Negative,
                match_type => {
                    if match_state != MatchType::Positive && match_state != MatchType::Negative {
                        match_state = match_type;
                    }
                    child_pattern.push(pattern.get_rest_pattern());
                }
            }
        }

        Ok((match_state, child_pattern))
    }
}
