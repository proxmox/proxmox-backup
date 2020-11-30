//! Code for extraction of pxar contents onto the file system.

use std::convert::TryFrom;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::path::Path;
use std::sync::{Arc, Mutex};

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
use crate::pxar::metadata;
use crate::pxar::Flags;

pub fn extract_archive<T, F>(
    mut decoder: pxar::decoder::Decoder<T>,
    destination: &Path,
    match_list: &[MatchEntry],
    extract_match_default: bool,
    feature_flags: Flags,
    allow_existing_dirs: bool,
    mut callback: F,
    on_error: Option<Box<dyn FnMut(Error) -> Result<(), Error> + Send>>,
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

    let mut extractor = Extractor::new(
        dir,
        root.metadata().clone(),
        allow_existing_dirs,
        feature_flags,
    );

    if let Some(on_error) = on_error {
        extractor.on_error(on_error);
    }

    let mut match_stack = Vec::new();
    let mut err_path_stack = vec![OsString::from("/")];
    let mut current_match = extract_match_default;
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

        extractor.set_path(entry.path().as_os_str().to_owned());

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
                callback(entry.path());

                let create = current_match && match_result != Some(MatchType::Exclude);
                extractor
                    .enter_directory(file_name_os.to_owned(), metadata.clone(), create)
                    .map_err(|err| format_err!("error at entry {:?}: {}", file_name_os, err))?;

                // We're starting a new directory, push our old matching state and replace it with
                // our new one:
                match_stack.push(current_match);
                current_match = did_match;

                // When we hit the goodbye table we'll try to apply metadata to the directory, but
                // the Goodbye entry will not contain the path, so push it to our path stack for
                // error messages:
                err_path_stack.push(extractor.clone_path());

                Ok(())
            }
            (_, EntryKind::GoodbyeTable) => {
                // go up a directory

                extractor.set_path(err_path_stack.pop().ok_or_else(|| {
                    format_err!(
                        "error at entry {:?}: unexpected end of directory",
                        file_name_os
                    )
                })?);

                extractor
                    .leave_directory()
                    .map_err(|err| format_err!("error at entry {:?}: {}", file_name_os, err))?;

                // We left a directory, also get back our previous matching state. This is in sync
                // with `dir_stack` so this should never be empty except for the final goodbye
                // table, in which case we get back to the default of `true`.
                current_match = match_stack.pop().unwrap_or(true);

                Ok(())
            }
            (true, EntryKind::Symlink(link)) => {
                callback(entry.path());
                extractor.extract_symlink(&file_name, metadata, link.as_ref())
            }
            (true, EntryKind::Hardlink(link)) => {
                callback(entry.path());
                extractor.extract_hardlink(&file_name, link.as_os_str())
            }
            (true, EntryKind::Device(dev)) => {
                if extractor.contains_flags(Flags::WITH_DEVICE_NODES) {
                    callback(entry.path());
                    extractor.extract_device(&file_name, metadata, dev)
                } else {
                    Ok(())
                }
            }
            (true, EntryKind::Fifo) => {
                if extractor.contains_flags(Flags::WITH_FIFOS) {
                    callback(entry.path());
                    extractor.extract_special(&file_name, metadata, 0)
                } else {
                    Ok(())
                }
            }
            (true, EntryKind::Socket) => {
                if extractor.contains_flags(Flags::WITH_SOCKETS) {
                    callback(entry.path());
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

/// Common state for file extraction.
pub(crate) struct Extractor {
    feature_flags: Flags,
    allow_existing_dirs: bool,
    dir_stack: PxarDirStack,

    /// For better error output we need to track the current path in the Extractor state.
    current_path: Arc<Mutex<OsString>>,

    /// Error callback. Includes `current_path` in the reformatted error, should return `Ok` to
    /// continue extracting or the passed error as `Err` to bail out.
    on_error: Box<dyn FnMut(Error) -> Result<(), Error> + Send>,
}

impl Extractor {
    /// Create a new extractor state for a target directory.
    pub fn new(
        root_dir: Dir,
        metadata: Metadata,
        allow_existing_dirs: bool,
        feature_flags: Flags,
    ) -> Self {
        Self {
            dir_stack: PxarDirStack::new(root_dir, metadata),
            allow_existing_dirs,
            feature_flags,
            current_path: Arc::new(Mutex::new(OsString::new())),
            on_error: Box::new(|err| Err(err)),
        }
    }

    /// We call this on errors. The error will be reformatted to include `current_path`. The
    /// callback should decide whether this error was fatal (simply return it) to bail out early,
    /// or log/remember/accumulate errors somewhere and return `Ok(())` in its place to continue
    /// extracting.
    pub fn on_error(&mut self, mut on_error: Box<dyn FnMut(Error) -> Result<(), Error> + Send>) {
        let path = Arc::clone(&self.current_path);
        self.on_error = Box::new(move |err: Error| -> Result<(), Error> {
            on_error(format_err!("error at {:?}: {}", path.lock().unwrap(), err))
        });
    }

    pub fn set_path(&mut self, path: OsString) {
        *self.current_path.lock().unwrap() = path;
    }

    pub fn clone_path(&self) -> OsString {
        self.current_path.lock().unwrap().clone()
    }

    /// When encountering a directory during extraction, this is used to keep track of it. If
    /// `create` is true it is immediately created and its metadata will be updated once we leave
    /// it. If `create` is false it will only be created if it is going to have any actual content.
    pub fn enter_directory(
        &mut self,
        file_name: OsString,
        metadata: Metadata,
        create: bool,
    ) -> Result<(), Error> {
        self.dir_stack.push(file_name, metadata)?;

        if create {
            self.dir_stack.create_last_dir(self.allow_existing_dirs)?;
        }

        Ok(())
    }

    /// When done with a directory we can apply its metadata if it has been created.
    pub fn leave_directory(&mut self) -> Result<(), Error> {
        let dir = self
            .dir_stack
            .pop()
            .map_err(|err| format_err!("unexpected end of directory entry: {}", err))?
            .ok_or_else(|| format_err!("broken pxar archive (directory stack underrun)"))?;

        if let Some(fd) = dir.try_as_borrowed_fd() {
            metadata::apply(
                self.feature_flags,
                dir.metadata(),
                fd.as_raw_fd(),
                &CString::new(dir.file_name().as_bytes())?,
                &mut self.on_error,
            )
            .map_err(|err| format_err!("failed to apply directory metadata: {}", err))?;
        }

        Ok(())
    }

    fn contains_flags(&self, flag: Flags) -> bool {
        self.feature_flags.contains(flag)
    }

    fn parent_fd(&mut self) -> Result<RawFd, Error> {
        self.dir_stack
            .last_dir_fd(self.allow_existing_dirs)
            .map(|d| d.as_raw_fd())
            .map_err(|err| format_err!("failed to get parent directory file descriptor: {}", err))
    }

    pub fn extract_symlink(
        &mut self,
        file_name: &CStr,
        metadata: &Metadata,
        link: &OsStr,
    ) -> Result<(), Error> {
        let parent = self.parent_fd()?;
        nix::unistd::symlinkat(link, Some(parent), file_name)?;
        metadata::apply_at(
            self.feature_flags,
            metadata,
            parent,
            file_name,
            &mut self.on_error,
        )
    }

    pub fn extract_hardlink(&mut self, file_name: &CStr, link: &OsStr) -> Result<(), Error> {
        crate::pxar::tools::assert_relative_path(link)?;

        let parent = self.parent_fd()?;
        let root = self.dir_stack.root_dir_fd()?;
        let target = CString::new(link.as_bytes())?;
        nix::unistd::linkat(
            Some(root.as_raw_fd()),
            target.as_c_str(),
            Some(parent),
            file_name,
            nix::unistd::LinkatFlags::NoSymlinkFollow,
        )?;

        Ok(())
    }

    pub fn extract_device(
        &mut self,
        file_name: &CStr,
        metadata: &Metadata,
        device: &Device,
    ) -> Result<(), Error> {
        self.extract_special(file_name, metadata, device.to_dev_t())
    }

    pub fn extract_special(
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

        metadata::apply_at(
            self.feature_flags,
            metadata,
            parent,
            file_name,
            &mut self.on_error,
        )
    }

    pub fn extract_file(
        &mut self,
        file_name: &CStr,
        metadata: &Metadata,
        size: u64,
        contents: &mut dyn io::Read,
    ) -> Result<(), Error> {
        let parent = self.parent_fd()?;
        let mut file = unsafe {
            std::fs::File::from_raw_fd(
                nix::fcntl::openat(
                    parent,
                    file_name,
                    OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_WRONLY | OFlag::O_CLOEXEC,
                    Mode::from_bits(0o600).unwrap(),
                )
                .map_err(|err| format_err!("failed to create file {:?}: {}", file_name, err))?,
            )
        };

        metadata::apply_initial_flags(
            self.feature_flags,
            metadata,
            file.as_raw_fd(),
            &mut self.on_error,
        )?;

        let extracted = io::copy(&mut *contents, &mut file)
            .map_err(|err| format_err!("failed to copy file contents: {}", err))?;
        if size != extracted {
            bail!("extracted {} bytes of a file of {} bytes", extracted, size);
        }

        metadata::apply(
            self.feature_flags,
            metadata,
            file.as_raw_fd(),
            file_name,
            &mut self.on_error,
        )
    }

    pub async fn async_extract_file<T: tokio::io::AsyncRead + Unpin>(
        &mut self,
        file_name: &CStr,
        metadata: &Metadata,
        size: u64,
        contents: &mut T,
    ) -> Result<(), Error> {
        let parent = self.parent_fd()?;
        let mut file = tokio::fs::File::from_std(unsafe {
            std::fs::File::from_raw_fd(
                nix::fcntl::openat(
                    parent,
                    file_name,
                    OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_WRONLY | OFlag::O_CLOEXEC,
                    Mode::from_bits(0o600).unwrap(),
                )
                .map_err(|err| format_err!("failed to create file {:?}: {}", file_name, err))?,
            )
        });

        metadata::apply_initial_flags(
            self.feature_flags,
            metadata,
            file.as_raw_fd(),
            &mut self.on_error,
        )?;

        let extracted = tokio::io::copy(&mut *contents, &mut file)
            .await
            .map_err(|err| format_err!("failed to copy file contents: {}", err))?;
        if size != extracted {
            bail!("extracted {} bytes of a file of {} bytes", extracted, size);
        }

        metadata::apply(
            self.feature_flags,
            metadata,
            file.as_raw_fd(),
            file_name,
            &mut self.on_error,
        )
    }
}
