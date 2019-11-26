use std::collections::HashSet;
use std::ffi::{CStr, CString, OsStr};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use failure::*;
use libc;

use crate::pxar::*;

use super::catalog::{CatalogReader, DirEntry};
use super::readline::{Readline, Context};

/// State of the shell instance
pub struct Shell {
    /// Readline context
    rl: Readline,
    /// List of paths selected for a restore
    selected: HashSet<Vec<u8>>,
    /// Decoder instance for the current pxar archive
    decoder: Decoder,
    /// Root directory for the give archive as stored in the catalog
    root: Vec<DirEntry>,
}

/// All supported commands of the shell
enum Command<'a> {
    /// List the content of the current dir or in path, if provided
    List(&'a [u8]),
    /// Stat of the provided path
    Stat(&'a [u8]),
    /// Select the given entry for a restore
    Select(&'a [u8]),
    /// Remove the entry from the list of entries to restore
    Deselect(&'a [u8]),
    /// Restore an archive to the provided target, can be limited to files
    /// matching the provided match pattern
    Restore(&'a [u8], &'a [u8]),
    /// Restore the selected entries to the provided target
    RestoreSelected(&'a [u8]),
    /// List the entries currently selected for restore
    ListSelected,
    /// Change the current working directory
    ChangeDir(&'a [u8]),
    /// Print the working directory
    PrintWorkingDir,
    /// Terminate the shell loop, returns from the shell
    Quit,
    /// Empty line from readline
    Empty,
}

const PROMPT_PREFIX: &str = "pxar:";
const PROMPT_POST: &str = " > ";

impl Shell {
    pub fn new(
        mut catalog: CatalogReader<std::fs::File>,
        archive_name: &str,
        decoder: Decoder,
    ) -> Result<Self, Error> {
        let catalog_root = catalog.root()?;
        // The root for the given archive as stored in the catalog
        let archive_root = catalog.lookup(&catalog_root, archive_name.as_bytes())?;
        let root = vec![archive_root];
        Ok(Self {
            rl: Readline::new(
                Self::generate_prompt(b"/"),
                root.clone(),
                Box::new(complete),
                catalog,
            ),
            selected: HashSet::new(),
            decoder,
            root,
        })
    }

    fn generate_prompt(path: &[u8]) -> CString {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(PROMPT_PREFIX.as_bytes());
        buffer.extend_from_slice(path);
        buffer.extend_from_slice(PROMPT_POST.as_bytes());
        unsafe { CString::from_vec_unchecked(buffer) }
    }

    /// Start the interactive shell loop
    pub fn shell(mut self) -> Result<(), Error> {
        while let Some(line) = self.rl.readline() {
            let res = match self.parse_command(&line) {
                Ok(Command::List(path)) => self.list(path).and_then(|list| {
                    Self::print_list(&list).map_err(|err| format_err!("{}", err))?;
                    Ok(())
                }),
                Ok(Command::ChangeDir(path)) => self.change_dir(path),
                Ok(Command::Restore(target, pattern)) => self.restore(target, pattern),
                Ok(Command::Select(path)) => self.select(path),
                Ok(Command::Deselect(path)) => self.deselect(path),
                Ok(Command::RestoreSelected(target)) => self.restore_selected(target),
                Ok(Command::Stat(path)) => self.stat(&path).and_then(|(item, attr, size)| {
                    Self::print_stat(&item, &attr, size)?;
                    Ok(())
                }),
                Ok(Command::ListSelected) => {
                    self.list_selected().map_err(|err| format_err!("{}", err))
                }
                Ok(Command::PrintWorkingDir) => self.pwd().and_then(|pwd| {
                    Self::print_pwd(&pwd).map_err(|err| format_err!("{}", err))?;
                    Ok(())
                }),
                Ok(Command::Quit) => break,
                Ok(Command::Empty) => continue,
                Err(err) => Err(err),
            };
            if let Err(err) = res {
                println!("error: {}", err);
            }
        }
        Ok(())
    }

    /// Command parser mapping the line returned by readline to a command.
    fn parse_command<'a>(&self, line: &'a [u8]) -> Result<Command<'a>, Error> {
        // readline already handles tabs, so here we only split on spaces
        let args: Vec<&[u8]> = line
            .split(|b| *b == b' ')
            .filter(|word| !word.is_empty())
            .collect();

        if args.is_empty() {
            return Ok(Command::Empty);
        }

        match args[0] {
            b"quit" => Ok(Command::Quit),
            b"exit" => Ok(Command::Quit),
            b"ls" => match args.len() {
                1 => Ok(Command::List(&[])),
                2 => Ok(Command::List(args[1])),
                _ => bail!("To many parameters!"),
            },
            b"pwd" => Ok(Command::PrintWorkingDir),
            b"restore" => match args.len() {
                1 => bail!("no target provided"),
                2 => Ok(Command::Restore(args[1], &[])),
                4 => if args[2] == b"-p" {
                    Ok(Command::Restore(args[1], args[3]))
                } else {
                    bail!("invalid parameter")
                }
                _ => bail!("to many parameters"),
            },
            b"cd" => match args.len() {
                1 => Ok(Command::ChangeDir(&[])),
                2 => Ok(Command::ChangeDir(args[1])),
                _ => bail!("to many parameters"),
            },
            b"stat" => match args.len() {
                1 => bail!("no path provided"),
                2 => Ok(Command::Stat(args[1])),
                _ => bail!("to many parameters"),
            },
            b"select" => match args.len() {
                1 => bail!("no path provided"),
                2 => Ok(Command::Select(args[1])),
                _ => bail!("to many parameters"),
            },
            b"deselect" => match args.len() {
                1 => bail!("no path provided"),
                2 => Ok(Command::Deselect(args[1])),
                _ => bail!("to many parameters"),
            },
            b"selected" => match args.len() {
                1 => Ok(Command::ListSelected),
                _ => bail!("to many parameters"),
            },
            b"restore-selected" => match args.len() {
                1 => bail!("no path provided"),
                2 => Ok(Command::RestoreSelected(args[1])),
                _ => bail!("to many parameters"),
            },
            _ => bail!("command not known"),
        }
    }

    /// Get a mut ref to the context in order to be able to access the
    /// catalog and the directory stack for the current working directory.
    fn context(&mut self) -> &mut Context {
        self.rl.context()
    }

    /// Change the current working directory to the new directory
    fn change_dir(&mut self, path: &[u8]) -> Result<(), Error> {
        let mut path = self.canonical_path(path)?;
        if !path
            .last()
            .ok_or_else(|| format_err!("invalid path component"))?
            .is_directory()
        {
            // Change to the parent dir of the file instead
            path.pop();
            eprintln!("not a directory, fallback to parent directory");
        }
        self.context().current = path;
        // Update the directory displayed in the prompt
        let prompt = Self::generate_prompt(self.pwd()?.as_slice());
        self.rl.update_prompt(prompt);
        Ok(())
    }

    /// List the content of a directory.
    ///
    /// Executed on files it returns the DirEntry of the file as single element
    /// in the list.
    fn list(&mut self, path: &[u8]) -> Result<Vec<DirEntry>, Error> {
        let parent = if !path.is_empty() {
            self.canonical_path(path)?
                .last()
                .ok_or_else(|| format_err!("invalid path component"))?
                .clone()
        } else {
            self.context().current.last().unwrap().clone()
        };

        let list = if parent.is_directory() {
            self.context().catalog.read_dir(&parent)?
        } else {
            vec![parent]
        };
        Ok(list)
    }

    /// Return the current working directory as string
    fn pwd(&mut self) -> Result<Vec<u8>, Error> {
        Self::to_path(&self.context().current.clone())
    }

    /// Generate an absolute path from a directory stack.
    fn to_path(dir_stack: &[DirEntry]) -> Result<Vec<u8>, Error> {
        let mut path = vec![b'/'];
        // Skip the archive root, '/' is displayed for it
        for item in dir_stack.iter().skip(1) {
            path.extend_from_slice(&item.name);
            if item.is_directory() {
                path.push(b'/');
            }
        }
        Ok(path)
    }

    /// Resolve the indirect path components and return an absolute path.
    ///
    /// This will actually navigate the filesystem tree to check that the
    /// path is vaild and exists.
    /// This does not include following symbolic links.
    /// If None is given as path, only the root directory is returned.
    fn canonical_path(&mut self, path: &[u8]) -> Result<Vec<DirEntry>, Error> {
        if path == b"/" {
            return Ok(self.root.clone());
        }

        let mut path_slice = if path.is_empty() {
            // Fallback to root if no path was provided
            return Ok(self.root.clone());
        } else {
            path
        };

        let mut dir_stack = if path_slice.starts_with(&[b'/']) {
            // Absolute path, reduce view of slice and start from root
            path_slice = &path_slice[1..];
            self.root.clone()
        } else {
            // Relative path, start from current working directory
            self.context().current.clone()
        };
        let should_end_dir = if path_slice.ends_with(&[b'/']) {
            path_slice = &path_slice[0..path_slice.len() - 1];
            true
        } else {
            false
        };
        for name in path_slice.split(|b| *b == b'/') {
            match name {
                b"." => continue,
                b".." => {
                    // Never pop archive root from stack
                    if dir_stack.len() > 1 {
                        dir_stack.pop();
                    }
                }
                _ => {
                    let entry = self.context().catalog.lookup(dir_stack.last().unwrap(), name)?;
                    dir_stack.push(entry);
                }
            }
        }
        if should_end_dir && !dir_stack.last()
            .ok_or_else(|| format_err!("invalid path component"))?
            .is_directory()
        {
            bail!("entry is not a directory");
        }

        Ok(dir_stack)
    }

    /// Read the metadata for a given directory entry.
    ///
    /// This is expensive because the data has to be read from the pxar `Decoder`,
    /// which means reading over the network.
    fn stat(&mut self, path: &[u8]) -> Result<(DirectoryEntry, PxarAttributes, u64), Error> {
        // First check if the file exists in the catalog, therefore avoiding
        // expensive calls to the decoder just to find out that there could be no
        // such entry. This is done by calling canonical_path(), which returns
        // the full path if it exists, error otherwise.
        let path = self.canonical_path(path)?;
        self.lookup(&path)
    }

    /// Look up the entry given by a canonical absolute `path` in the archive.
    ///
    /// This will actively navigate the archive by calling the corresponding decoder
    /// functionalities and is therefore very expensive.
    fn lookup(
        &mut self,
        absolute_path: &[DirEntry],
    ) -> Result<(DirectoryEntry, PxarAttributes, u64), Error> {
        let mut current = self.decoder.root()?;
        let (_, _, mut attr, mut size) = self.decoder.attributes(0)?;
        // Ignore the archive root, don't need it.
        for item in absolute_path.iter().skip(1) {
            match self.decoder.lookup(&current, &OsStr::from_bytes(&item.name))? {
                Some((item, item_attr, item_size)) => {
                    current = item;
                    attr = item_attr;
                    size = item_size;
                }
                // This should not happen if catalog an archive are consistent.
                None => bail!("no such file or directory in archive"),
            }
        }
        Ok((current, attr, size))
    }

    /// Select an entry for restore.
    ///
    /// This will return an error if the entry is already present in the list or
    /// if an invalid path was provided.
    fn select(&mut self, path: &[u8]) -> Result<(), Error> {
        // Calling canonical_path() makes sure the provided path is valid and
        // actually contained within the catalog and therefore also the archive.
        let path = self.canonical_path(path)?;
        if self.selected.insert(Self::to_path(&path)?) {
            Ok(())
        } else {
            bail!("entry already selected for restore")
        }
    }

    /// Deselect an entry for restore.
    ///
    /// This will return an error if the entry was not found in the list of entries
    /// selected for restore.
    fn deselect(&mut self, path: &[u8]) -> Result<(), Error> {
        if self.selected.remove(path) {
            Ok(())
        } else {
            bail!("entry not selected for restore")
        }
    }

    /// Restore the selected entries to the given target path.
    ///
    /// Target must not exist on the clients filesystem.
    fn restore_selected(&mut self, target: &[u8]) -> Result<(), Error> {
        let mut list = Vec::new();
        for path in &self.selected {
            let pattern = MatchPattern::from_line(path)?
                .ok_or_else(|| format_err!("encountered invalid match pattern"))?;
            list.push(pattern);
        }

        // Entry point for the restore is always root here as the provided match
        // patterns are relative to root as well.
        let start_dir = self.decoder.root()?;
        let target: &OsStr = OsStrExt::from_bytes(target);
        self.decoder.restore(&start_dir, &Path::new(target), &list)?;
        Ok(())
    }

    /// List entries currently selected for restore.
    fn list_selected(&self) -> Result<(), std::io::Error> {
        let mut out = std::io::stdout();
        for entry in &self.selected {
            out.write_all(entry)?;
            out.write_all(&[b'\n'])?;
        }
        out.flush()?;
        Ok(())
    }

    /// Restore the sub-archive given by the current working directory to target.
    ///
    /// By further providing a pattern, the restore can be limited to a narrower
    /// subset of this sub-archive.
    /// If pattern is an empty slice, the full dir is restored.
    fn restore(&mut self, target: &[u8], pattern: &[u8]) -> Result<(), Error> {
        let match_pattern = match pattern {
            b"" | b"/" | b"." => Vec::new(),
            _ => vec![MatchPattern::from_line(pattern)?.unwrap()],
        };
        // Entry point for the restore.
        let start_dir = if pattern.starts_with(&[b'/']) {
            self.decoder.root()?
        } else {
            // Get the directory corresponding to the working directory from the
            // archive.
            let cwd = self.context().current.clone();
            let (dir, _, _) = self.lookup(&cwd)?;
            dir
        };

        let target: &OsStr = OsStrExt::from_bytes(target);
        self.decoder.restore(&start_dir, &Path::new(target), &match_pattern)?;
        Ok(())
    }

    fn print_list(list: &Vec<DirEntry>) -> Result<(), std::io::Error> {
        let max = list
            .iter()
            .max_by(|x, y| x.name.len().cmp(&y.name.len()));
        let max = match max {
            Some(dir_entry) => dir_entry.name.len() + 1,
            None => 0,
        };
        let (_rows, mut cols) = Self::get_terminal_size();
        cols /= max;
        let mut out = std::io::stdout();

        for (index, item) in list.iter().enumerate() {
            out.write_all(&item.name)?;
            // Fill with whitespaces
            out.write_all(&vec![b' '; max - item.name.len()])?;
            if index % cols == (cols - 1)  {
                out.write_all(&[b'\n'])?;
            }
        }
        // If the last line is not complete, add the newline
        if list.len() % cols != cols - 1 {
            out.write_all(&[b'\n'])?;
        }
        out.flush()?;
        Ok(())
    }

    fn print_pwd(pwd: &[u8]) -> Result<(), std::io::Error> {
        let mut out = std::io::stdout();
        out.write_all(pwd)?;
        out.write_all(&[b'\n'])?;
        out.flush()?;
        Ok(())
    }

    fn print_stat(item: &DirectoryEntry, _attr: &PxarAttributes, size: u64) -> Result<(), std::io::Error> {
        let mut out = std::io::stdout();
        out.write_all("File: ".as_bytes())?;
        out.write_all(&item.filename.as_bytes())?;
        out.write_all(&[b'\n'])?;
        out.write_all(format!("Size: {}\n", size).as_bytes())?;
        let mode = match item.entry.mode as u32 & libc::S_IFMT {
            libc::S_IFDIR => "directory".as_bytes(),
            libc::S_IFREG => "regular file".as_bytes(),
            libc::S_IFLNK => "symbolic link".as_bytes(),
            libc::S_IFBLK => "block special file".as_bytes(),
            libc::S_IFCHR => "character special file".as_bytes(),
            _ => "unknown".as_bytes(),
        };
        out.write_all("Type: ".as_bytes())?;
        out.write_all(&mode)?;
        out.write_all(&[b'\n'])?;
        out.write_all(format!("Uid: {}\n", item.entry.uid).as_bytes())?;
        out.write_all(format!("Gid: {}\n", item.entry.gid).as_bytes())?;
        out.flush()?;
        Ok(())
    }

    /// Get the current size of the terminal
    ///
    /// uses tty_ioctl, see man tty_ioctl(2)
    fn get_terminal_size() -> (usize, usize) {

        const TIOCGWINSZ: libc::c_ulong = 0x00005413;

        #[repr(C)]
        struct WinSize {
            ws_row: libc::c_ushort,
            ws_col: libc::c_ushort,
            _ws_xpixel: libc::c_ushort, // unused
            _ws_ypixel: libc::c_ushort, // unused
        }

        let mut winsize = WinSize {
            ws_row: 0,
            ws_col: 0,
            _ws_xpixel: 0,
            _ws_ypixel: 0,
        };
        unsafe { libc::ioctl(libc::STDOUT_FILENO, TIOCGWINSZ, &mut winsize) };
        (winsize.ws_row as usize, winsize.ws_col as usize)
    }
}

/// Filename completion callback for the shell
// TODO: impl command completion. For now only filename completion.
fn complete(
    ctx: &mut Context,
    text: &CStr,
    _start: usize,
    _end: usize
) -> Vec<CString> {
    let slices: Vec<_> = text
        .to_bytes()
        .split(|b| *b == b'/')
        .collect();
    let to_complete = match slices.last() {
        Some(last) => last,
        None => return Vec::new(),
    };
    let mut current = ctx.current.clone();
    let (prefix, entries) = {
        let mut prefix = Vec::new();
        if slices.len() > 1 {
            for component in &slices[..slices.len() - 1] {
                if component == b"." {
                    continue;
                } else if component == b".." {
                    // Never leave the current archive in the catalog
                    if current.len() > 1 { current.pop(); }
                } else {
                    match ctx.catalog.lookup(current.last().unwrap(), component) {
                        Err(_) => return Vec::new(),
                        Ok(dir) => current.push(dir),
                    }
                }
                prefix.extend_from_slice(component);
                prefix.push(b'/');
            }
        }
        let entries = match ctx.catalog.read_dir(&current.last().unwrap()) {
            Ok(entries) => entries,
            Err(_) => return Vec::new(),
        };
        (prefix, entries)
    };
    // Create a list of completion strings which outlives this function
    let mut list = Vec::new();
    for entry in &entries {
        if entry.name.starts_with(to_complete) {
            let mut name_buf = prefix.clone();
            name_buf.extend_from_slice(&entry.name);
            if entry.is_directory() {
                name_buf.push(b'/');
            }
            let name = unsafe { CString::from_vec_unchecked(name_buf) };
            list.push(name);
        }
    }
    list
}
