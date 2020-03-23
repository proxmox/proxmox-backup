//! Code for extraction of pxar contents onto the file system.

use std::convert::TryFrom;
use std::ffi::{CStr, CString, OsStr};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::path::Path;

use anyhow::{bail, format_err, Error};
use nix::dir::Dir;
use nix::fcntl::OFlag;
use nix::sys::stat::Mode;

use pathpatterns::{MatchEntry, MatchList, MatchType};
use pxar::format::Device;
use pxar::Metadata;

use proxmox::c_result;
use proxmox::tools::fs::{create_path, CreateOptions};

use crate::pxar::dir_stack::PxarDirStack;
use crate::pxar::flags;
use crate::pxar::metadata;

struct Extractor<'a> {
    /// FIXME: use bitflags!() for feature_flags
    feature_flags: u64,
    allow_existing_dirs: bool,
    callback: &'a mut dyn FnMut(&Path),
    dir_stack: PxarDirStack,
}

impl<'a> Extractor<'a> {
    fn with_flag(&self, flag: u64) -> bool {
        flag == (self.feature_flags & flag)
    }
}

pub fn extract_archive<T, F>(
    mut decoder: pxar::decoder::Decoder<T>,
    destination: &Path,
    match_list: &[MatchEntry],
    feature_flags: u64,
    allow_existing_dirs: bool,
    mut callback: F,
) -> Result<(), Error>
where
    T: pxar::decoder::SeqRead,
    F: FnMut(&Path),
{
    // we use this to keep track of our directory-traversal
    decoder.enable_goodbye_entries(true);

    let root = decoder
        .next()
        .ok_or_else(|| format_err!("found empty pxar archive"))?
        .map_err(|err| format_err!("error reading pxar archive: {}", err))?;

    if !root.is_dir() {
        bail!("pxar archive does not start with a directory entry!");
    }

    create_path(
        &destination,
        None,
        Some(CreateOptions::new().perm(Mode::from_bits_truncate(0o700))),
    )
    .map_err(|err| format_err!("error creating directory {:?}: {}", destination, err))?;

    let dir = Dir::open(
        destination,
        OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .map_err(|err| format_err!("unable to open target directory {:?}: {}", destination, err,))?;

    let mut extractor = Extractor {
        feature_flags,
        allow_existing_dirs,
        callback: &mut callback,
        dir_stack: PxarDirStack::new(dir, root.metadata().clone()),
    };

    let mut match_stack = Vec::new();
    let mut current_match = true;
    while let Some(entry) = decoder.next() {
        use pxar::EntryKind;

        let entry = entry.map_err(|err| format_err!("error reading pxar archive: {}", err))?;

        let file_name_os = entry.file_name();

        // safety check: a file entry in an archive must never contain slashes:
        if file_name_os.as_bytes().contains(&b'/') {
            bail!("archive file entry contains slashes, which is invalid and a security concern");
        }

        let file_name = CString::new(file_name_os.as_bytes())
            .map_err(|_| format_err!("encountered file name with null-bytes"))?;

        let metadata = entry.metadata();

        let match_result = match_list.matches(
            entry.path().as_os_str().as_bytes(),
            Some(metadata.file_type() as u32),
        );

        let did_match = match match_result {
            Some(MatchType::Include) => true,
            Some(MatchType::Exclude) => false,
            None => current_match,
        };
        match (did_match, entry.kind()) {
            (_, EntryKind::Directory) => {
                extractor.callback(entry.path());

                extractor
                    .dir_stack
                    .push(file_name_os.to_owned(), metadata.clone())?;

                if current_match && match_result != Some(MatchType::Exclude) {
                    // We're currently in a positive match and this directory does not match an
                    // exclude entry, so make sure it is created:
                    let _ = extractor
                        .dir_stack
                        .last_dir_fd(extractor.allow_existing_dirs)
                        .map_err(|err| {
                            format_err!("error creating entry {:?}: {}", file_name_os, err)
                        })?;
                }

                // We're starting a new directory, push our old matching state and replace it with
                // our new one:
                match_stack.push(current_match);
                current_match = did_match;

                Ok(())
            }
            (_, EntryKind::GoodbyeTable) => {
                // go up a directory
                let dir = extractor
                    .dir_stack
                    .pop()
                    .map_err(|err| format_err!("unexpected end of directory entry: {}", err))?
                    .ok_or_else(|| format_err!("broken pxar archive (directory stack underrun)"))?;
                // We left a directory, also get back our previous matching state. This is in sync
                // with `dir_stack` so this should never be empty except for the final goodbye
                // table, in which case we get back to the default of `true`.
                current_match = match_stack.pop().unwrap_or(true);

                if let Some(fd) = dir.try_as_raw_fd() {
                    metadata::apply(extractor.feature_flags, dir.metadata(), fd, &file_name)
                } else {
                    Ok(())
                }
            }
            (true, EntryKind::Symlink(link)) => {
                extractor.callback(entry.path());
                extractor.extract_symlink(&file_name, metadata, link.as_ref())
            }
            (true, EntryKind::Hardlink(link)) => {
                extractor.callback(entry.path());
                extractor.extract_hardlink(&file_name, metadata, link.as_os_str())
            }
            (true, EntryKind::Device(dev)) => {
                if extractor.with_flag(flags::WITH_DEVICE_NODES) {
                    extractor.callback(entry.path());
                    extractor.extract_device(&file_name, metadata, dev)
                } else {
                    Ok(())
                }
            }
            (true, EntryKind::Fifo) => {
                if extractor.with_flag(flags::WITH_FIFOS) {
                    extractor.callback(entry.path());
                    extractor.extract_special(&file_name, metadata, 0)
                } else {
                    Ok(())
                }
            }
            (true, EntryKind::Socket) => {
                if extractor.with_flag(flags::WITH_SOCKETS) {
                    extractor.callback(entry.path());
                    extractor.extract_special(&file_name, metadata, 0)
                } else {
                    Ok(())
                }
            }
            (true, EntryKind::File { size, .. }) => extractor.extract_file(
                &file_name,
                metadata,
                *size,
                &mut decoder.contents().ok_or_else(|| {
                    format_err!("found regular file entry without contents in archive")
                })?,
            ),
            (false, _) => Ok(()), // skip this
        }
        .map_err(|err| format_err!("error at entry {:?}: {}", file_name_os, err))?;
    }

    if !extractor.dir_stack.is_empty() {
        bail!("unexpected eof while decoding pxar archive");
    }

    Ok(())
}

impl<'a> Extractor<'a> {
    fn parent_fd(&mut self) -> Result<RawFd, Error> {
        self.dir_stack.last_dir_fd(self.allow_existing_dirs)
    }

    fn callback(&mut self, path: &Path) {
        (self.callback)(path)
    }

    fn extract_symlink(
        &mut self,
        file_name: &CStr,
        metadata: &Metadata,
        link: &OsStr,
    ) -> Result<(), Error> {
        let parent = self.parent_fd()?;
        nix::unistd::symlinkat(link, Some(parent), file_name)?;
        metadata::apply_at(self.feature_flags, metadata, parent, file_name)
    }

    fn extract_hardlink(
        &mut self,
        file_name: &CStr,
        _metadata: &Metadata, // for now we don't use this because hardlinks don't need it...
        link: &OsStr,
    ) -> Result<(), Error> {
        crate::pxar::tools::assert_relative_path(link)?;

        let parent = self.parent_fd()?;
        let root = self.dir_stack.root_dir_fd()?;
        let target = CString::new(link.as_bytes())?;
        nix::unistd::linkat(
            Some(root),
            target.as_c_str(),
            Some(parent),
            file_name,
            nix::unistd::LinkatFlags::NoSymlinkFollow,
        )?;

        Ok(())
    }

    fn extract_device(
        &mut self,
        file_name: &CStr,
        metadata: &Metadata,
        device: &Device,
    ) -> Result<(), Error> {
        self.extract_special(file_name, metadata, device.to_dev_t())
    }

    fn extract_special(
        &mut self,
        file_name: &CStr,
        metadata: &Metadata,
        device: libc::dev_t,
    ) -> Result<(), Error> {
        let mode = metadata.stat.mode;
        let mode = u32::try_from(mode).map_err(|_| {
            format_err!(
                "device node's mode contains illegal bits: 0x{:x} (0o{:o})",
                mode,
                mode,
            )
        })?;
        let parent = self.parent_fd()?;
        unsafe { c_result!(libc::mknodat(parent, file_name.as_ptr(), mode, device)) }
            .map_err(|err| format_err!("failed to create device node: {}", err))?;

        metadata::apply_at(self.feature_flags, metadata, parent, file_name)
    }

    fn extract_file(
        &mut self,
        file_name: &CStr,
        metadata: &Metadata,
        size: u64,
        contents: &mut dyn io::Read,
    ) -> Result<(), Error> {
        let parent = self.parent_fd()?;
        let mut file = unsafe {
            std::fs::File::from_raw_fd(nix::fcntl::openat(
                parent,
                file_name,
                OFlag::O_CREAT | OFlag::O_WRONLY | OFlag::O_CLOEXEC,
                Mode::from_bits(0o600).unwrap(),
            )?)
        };

        let extracted = io::copy(&mut *contents, &mut file)?;
        if size != extracted {
            bail!("extracted {} bytes of a file of {} bytes", extracted, size);
        }

        metadata::apply(self.feature_flags, metadata, file.as_raw_fd(), file_name)
    }
}
