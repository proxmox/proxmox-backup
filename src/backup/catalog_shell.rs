use std::cell::RefCell;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::ffi::{CString, OsStr};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use chrono::{Utc, offset::TimeZone};
use anyhow::{bail, format_err, Error};
use nix::sys::stat::{Mode, SFlag};

use proxmox::api::{cli::*, *};
use proxmox::sys::linux::tty;

use super::catalog::{CatalogReader, DirEntry};
use crate::pxar::*;
use crate::tools;


const PROMPT_PREFIX: &str = "pxar:";
const PROMPT: &str = ">";

/// Interactive shell for interacton with the catalog.
pub struct Shell {
    /// Readline instance handling input and callbacks
    rl: rustyline::Editor<CliHelper>,
    prompt: String,
}

/// This list defines all the shell commands and their properties
/// using the api schema
pub fn catalog_shell_cli() -> CommandLineInterface {

    let map = CliCommandMap::new()
        .insert("pwd", CliCommand::new(&API_METHOD_PWD_COMMAND))
        .insert(
            "cd",
            CliCommand::new(&API_METHOD_CD_COMMAND)
                .arg_param(&["path"])
                .completion_cb("path", Shell::complete_path)
        )
        .insert(
            "ls",
            CliCommand::new(&API_METHOD_LS_COMMAND)
                .arg_param(&["path"])
                .completion_cb("path", Shell::complete_path)
         )
        .insert(
            "stat",
            CliCommand::new(&API_METHOD_STAT_COMMAND)
                .arg_param(&["path"])
                .completion_cb("path", Shell::complete_path)
         )
        .insert(
            "select",
            CliCommand::new(&API_METHOD_SELECT_COMMAND)
                .arg_param(&["path"])
                .completion_cb("path", Shell::complete_path)
        )
        .insert(
            "deselect",
            CliCommand::new(&API_METHOD_DESELECT_COMMAND)
                .arg_param(&["path"])
                .completion_cb("path", Shell::complete_path)
        )
        .insert(
            "clear-selected",
            CliCommand::new(&API_METHOD_CLEAR_SELECTED_COMMAND)
        )
        .insert(
            "restore-selected",
            CliCommand::new(&API_METHOD_RESTORE_SELECTED_COMMAND)
                .arg_param(&["target"])
                .completion_cb("target", tools::complete_file_name)
        )
        .insert(
            "list-selected",
            CliCommand::new(&API_METHOD_LIST_SELECTED_COMMAND),
        )
        .insert(
            "restore",
            CliCommand::new(&API_METHOD_RESTORE_COMMAND)
                .arg_param(&["target"])
                .completion_cb("target", tools::complete_file_name)
        )
        .insert(
            "find",
            CliCommand::new(&API_METHOD_FIND_COMMAND)
                .arg_param(&["path", "pattern"])
                .completion_cb("path", Shell::complete_path)
        )
        .insert_help();

    CommandLineInterface::Nested(map)
}

impl Shell {
    /// Create a new shell for the given catalog and pxar archive.
    pub fn new(
        mut catalog: CatalogReader<std::fs::File>,
        archive_name: &str,
        decoder: Decoder,
    ) -> Result<Self, Error> {
        let catalog_root = catalog.root()?;
        // The root for the given archive as stored in the catalog
        let archive_root = catalog.lookup(&catalog_root, archive_name.as_bytes())?;
        let path = CatalogPathStack::new(archive_root);

        CONTEXT.with(|handle| {
            let mut ctx = handle.borrow_mut();
            *ctx = Some(Context {
                catalog,
                selected: Vec::new(),
                decoder,
                path,
            });
        });

        let cli_helper = CliHelper::new(catalog_shell_cli());
        let mut rl = rustyline::Editor::<CliHelper>::new();
        rl.set_helper(Some(cli_helper));

        Context::with(|ctx| {
            Ok(Self {
                rl,
                prompt: ctx.generate_prompt()?,
            })
        })
    }

    /// Start the interactive shell loop
    pub fn shell(mut self) -> Result<(), Error> {
        while let Ok(line) = self.rl.readline(&self.prompt) {
            let helper = self.rl.helper().unwrap();
            let args = match shellword_split(&line) {
                Ok(args) => args,
                Err(err) => {
                    println!("Error: {}", err);
                    continue;
                }
            };
            let _ = handle_command(helper.cmd_def(), "", args, None);
            self.rl.add_history_entry(line);
            self.update_prompt()?;
        }
        Ok(())
    }

    /// Update the prompt to the new working directory
    fn update_prompt(&mut self) -> Result<(), Error> {
        Context::with(|ctx| {
            self.prompt = ctx.generate_prompt()?;
            Ok(())
        })
    }

    /// Completions for paths by lookup in the catalog
    fn complete_path(complete_me: &str, _map: &HashMap<String, String>) -> Vec<String> {
        Context::with(|ctx| {
            let (base, to_complete) = match complete_me.rfind('/') {
                // Split at ind + 1 so the slash remains on base, ok also if
                // ends in slash as split_at accepts up to length as index.
                Some(ind) => complete_me.split_at(ind + 1),
                None => ("", complete_me),
            };

            let current = if base.is_empty() {
                ctx.path.last().clone()
            } else {
                let mut local = ctx.path.clone();
                local.traverse(&PathBuf::from(base), &mut ctx.decoder, &mut ctx.catalog, false)?;
                local.last().clone()
            };

            let entries = match ctx.catalog.read_dir(&current) {
                Ok(entries) => entries,
                Err(_) => return Ok(Vec::new()),
            };

            let mut list = Vec::new();
            for entry in &entries {
                let mut name = String::from(base);
                if entry.name.starts_with(to_complete.as_bytes()) {
                    name.push_str(std::str::from_utf8(&entry.name)?);
                    if entry.is_directory() {
                        name.push('/');
                    }
                    list.push(name);
                }
            }
            Ok(list)
        })
        .unwrap_or_default()
    }
}

#[api(input: { properties: {} })]
/// List the current working directory.
fn pwd_command() -> Result<(), Error> {
    Context::with(|ctx| {
        let path = ctx.path.generate_cstring()?;
        let mut out = std::io::stdout();
        out.write_all(&path.as_bytes())?;
        out.write_all(&[b'\n'])?;
        out.flush()?;
        Ok(())
    })
}

#[api(
    input: {
        properties: {
            path: {
                type: String,
                optional: true,
                description: "target path."
            }
        }
    }
)]
/// Change the current working directory to the new directory
fn cd_command(path: Option<String>) -> Result<(), Error> {
    Context::with(|ctx| {
        let path = path.unwrap_or_default();
        if path.is_empty() {
            ctx.path.clear();
            return Ok(());
        }
        let mut local = ctx.path.clone();
        local.traverse(&PathBuf::from(path), &mut ctx.decoder, &mut ctx.catalog, true)?;
        if !local.last().is_directory() {
            local.pop();
            eprintln!("not a directory, fallback to parent directory");
        }
        ctx.path = local;
        Ok(())
    })
}

#[api(
    input: {
        properties: {
            path: {
                type: String,
                optional: true,
                description: "target path."
            }
        }
    }
)]
/// List the content of working directory or given path.
fn ls_command(path: Option<String>) -> Result<(), Error> {
    Context::with(|ctx| {
        let parent = if let Some(ref path) = path {
            let mut local = ctx.path.clone();
            local.traverse(&PathBuf::from(path), &mut ctx.decoder, &mut ctx.catalog, false)?;
            local.last().clone()
        } else {
            ctx.path.last().clone()
        };

        let list = if parent.is_directory() {
            ctx.catalog.read_dir(&parent)?
        } else {
            vec![parent.clone()]
        };

        if list.is_empty() {
            return Ok(());
        }
        let max = list.iter().max_by(|x, y| x.name.len().cmp(&y.name.len()));
        let max = match max {
            Some(dir_entry) => dir_entry.name.len() + 1,
            None => 0,
        };

        let (_rows, mut cols) = tty::stdout_terminal_size();
        cols /= max;

        let mut out = std::io::stdout();
        for (index, item) in list.iter().enumerate() {
            out.write_all(&item.name)?;
            // Fill with whitespaces
            out.write_all(&vec![b' '; max - item.name.len()])?;
            if index % cols == (cols - 1) {
                out.write_all(&[b'\n'])?;
            }
        }
        // If the last line is not complete, add the newline
        if list.len() % cols != cols - 1 {
            out.write_all(&[b'\n'])?;
        }
        out.flush()?;
        Ok(())
    })
}

#[api(
    input: {
        properties: {
            path: {
                type: String,
                description: "target path."
            }
        }
    }
)]
/// Read the metadata for a given directory entry.
///
/// This is expensive because the data has to be read from the pxar `Decoder`,
/// which means reading over the network.
fn stat_command(path: String) -> Result<(), Error> {
    Context::with(|ctx| {
        let mut local = ctx.path.clone();
        local.traverse(&PathBuf::from(path), &mut ctx.decoder, &mut ctx.catalog, false)?;
        let canonical = local.canonical(&mut ctx.decoder, &mut ctx.catalog, false)?;
        let item = canonical.lookup(&mut ctx.decoder)?;
        let mut out = std::io::stdout();
        out.write_all(b"  File:\t")?;
        out.write_all(item.filename.as_bytes())?;
        out.write_all(b"\n")?;
        out.write_all(format!("  Size:\t{}\t\t", item.size).as_bytes())?;
        out.write_all(b"Type:\t")?;

        let mut mode_out = vec![b'-'; 10];
        match SFlag::from_bits_truncate(item.entry.mode as u32) {
            SFlag::S_IFDIR => {
                mode_out[0] = b'd';
                out.write_all(b"directory\n")?;
            }
            SFlag::S_IFREG => {
                mode_out[0] = b'-';
                out.write_all(b"regular file\n")?;
            }
            SFlag::S_IFLNK => {
                mode_out[0] = b'l';
                out.write_all(b"symbolic link\n")?;
            }
            SFlag::S_IFBLK => {
                mode_out[0] = b'b';
                out.write_all(b"block special file\n")?;
            }
            SFlag::S_IFCHR => {
                mode_out[0] = b'c';
                out.write_all(b"character special file\n")?;
            }
            _ => out.write_all(b"unknown\n")?,
        };

        let mode = Mode::from_bits_truncate(item.entry.mode as u32);
        if mode.contains(Mode::S_IRUSR) {
            mode_out[1] = b'r';
        }
        if mode.contains(Mode::S_IWUSR) {
            mode_out[2] = b'w';
        }
        match (mode.contains(Mode::S_IXUSR), mode.contains(Mode::S_ISUID)) {
            (false, false) => mode_out[3] = b'-',
            (true, false) => mode_out[3] = b'x',
            (false, true) => mode_out[3] = b'S',
            (true, true) => mode_out[3] = b's',
        }

        if mode.contains(Mode::S_IRGRP) {
            mode_out[4] = b'r';
        }
        if mode.contains(Mode::S_IWGRP) {
            mode_out[5] = b'w';
        }
        match (mode.contains(Mode::S_IXGRP), mode.contains(Mode::S_ISGID)) {
            (false, false) => mode_out[6] = b'-',
            (true, false) => mode_out[6] = b'x',
            (false, true) => mode_out[6] = b'S',
            (true, true) => mode_out[6] = b's',
        }

        if mode.contains(Mode::S_IROTH) {
            mode_out[7] = b'r';
        }
        if mode.contains(Mode::S_IWOTH) {
            mode_out[8] = b'w';
        }
        match (mode.contains(Mode::S_IXOTH), mode.contains(Mode::S_ISVTX)) {
            (false, false) => mode_out[9] = b'-',
            (true, false) => mode_out[9] = b'x',
            (false, true) => mode_out[9] = b'T',
            (true, true) => mode_out[9] = b't',
        }

        if !item.xattr.xattrs.is_empty() {
            mode_out.push(b'+');
        }

        out.write_all(b"Access:\t")?;
        out.write_all(&mode_out)?;
        out.write_all(b"\t")?;
        out.write_all(format!(" Uid:\t{}\t", item.entry.uid).as_bytes())?;
        out.write_all(format!("Gid:\t{}\n", item.entry.gid).as_bytes())?;

        let time = i64::try_from(item.entry.mtime)?;
        let sec = time / 1_000_000_000;
        let nsec = u32::try_from(time % 1_000_000_000)?;
        let dt = Utc.timestamp(sec, nsec);
        out.write_all(format!("Modify:\t{}\n", dt.to_rfc2822()).as_bytes())?;
        out.flush()?;
        Ok(())
    })
}

#[api(
    input: {
        properties: {
            path: {
                type: String,
                description: "target path."
            }
        }
    }
)]
/// Select an entry for restore.
///
/// This will return an error if the entry is already present in the list or
/// if an invalid path was provided.
fn select_command(path: String) -> Result<(), Error> {
    Context::with(|ctx| {
        let mut local = ctx.path.clone();
        local.traverse(&PathBuf::from(path), &mut ctx.decoder, &mut ctx.catalog, false)?;
        let canonical = local.canonical(&mut ctx.decoder, &mut ctx.catalog, false)?;
        let pattern = MatchPattern::from_line(canonical.generate_cstring()?.as_bytes())?
            .ok_or_else(|| format_err!("encountered invalid match pattern"))?;
        if ctx.selected.iter().find(|p| **p == pattern).is_none() {
            ctx.selected.push(pattern);
        }
        Ok(())
    })
}

#[api(
    input: {
        properties: {
            path: {
                type: String,
                description: "path to entry to remove from list."
            }
        }
    }
)]
/// Deselect an entry for restore.
///
/// This will return an error if the entry was not found in the list of entries
/// selected for restore.
fn deselect_command(path: String) -> Result<(), Error> {
    Context::with(|ctx| {
        let mut local = ctx.path.clone();
        local.traverse(&PathBuf::from(path), &mut ctx.decoder, &mut ctx.catalog, false)?;
        let canonical = local.canonical(&mut ctx.decoder, &mut ctx.catalog, false)?;
        println!("{:?}", canonical.generate_cstring()?);
        let mut pattern = MatchPattern::from_line(canonical.generate_cstring()?.as_bytes())?
            .ok_or_else(|| format_err!("encountered invalid match pattern"))?;
        if let Some(last) = ctx.selected.last() {
            if last == &pattern {
                ctx.selected.pop();
                return Ok(());
            }
        }
        pattern.invert();
        ctx.selected.push(pattern);
        Ok(())
    })
}

#[api( input: { properties: { } })]
/// Clear the list of files selected for restore.
fn clear_selected_command() -> Result<(), Error> {
    Context::with(|ctx| {
        ctx.selected.clear();
        Ok(())
    })
}

#[api(
    input: {
        properties: {
            target: {
                type: String,
                description: "target path for restore on local filesystem."
            }
        }
    }
)]
/// Restore the selected entries to the given target path.
///
/// Target must not exist on the clients filesystem.
fn restore_selected_command(target: String) -> Result<(), Error> {
    Context::with(|ctx| {
        if ctx.selected.is_empty() {
            bail!("no entries selected for restore");
        }

        // Entry point for the restore is always root here as the provided match
        // patterns are relative to root as well.
        let start_dir = ctx.decoder.root()?;
        ctx.decoder
            .restore(&start_dir, &Path::new(&target), &ctx.selected)?;
        Ok(())
    })
}

#[api(
    input: {
        properties: {
            pattern: {
                type: Boolean,
                description: "List match patterns instead of the matching files.",
                optional: true,
            }
        }
    }
)]
/// List entries currently selected for restore.
fn list_selected_command(pattern: Option<bool>) -> Result<(), Error> {
    Context::with(|ctx| {
        let mut out = std::io::stdout();
        if let Some(true) = pattern {
            out.write_all(&MatchPattern::to_bytes(ctx.selected.as_slice()))?;
        } else {
            let mut slices = Vec::with_capacity(ctx.selected.len());
            for pattern in &ctx.selected {
                slices.push(pattern.as_slice());
            }
            let mut dir_stack = vec![ctx.path.root()];
            ctx.catalog.find(
                &mut dir_stack,
                &slices,
                &Box::new(|path: &[DirEntry]| println!("{:?}", Context::generate_cstring(path).unwrap()))
            )?;
        }
        out.flush()?;
        Ok(())
    })
}

#[api(
    input: {
        properties: {
            target: {
                type: String,
                description: "target path for restore on local filesystem."
            },
            pattern: {
                type: String,
                optional: true,
                description: "match pattern to limit files for restore."
            }
        }
    }
)]
/// Restore the sub-archive given by the current working directory to target.
///
/// By further providing a pattern, the restore can be limited to a narrower
/// subset of this sub-archive.
/// If pattern is not present or empty, the full archive is restored to target.
fn restore_command(target: String, pattern: Option<String>) -> Result<(), Error> {
    Context::with(|ctx| {
        let pattern = pattern.unwrap_or_default();
        let match_pattern = match pattern.as_str() {
            "" | "/" | "." => Vec::new(),
            _ => vec![MatchPattern::from_line(pattern.as_bytes())?.unwrap()],
        };
        // Decoder entry point for the restore.
        let start_dir = if pattern.starts_with("/") {
            ctx.decoder.root()?
        } else {
            // Get the directory corresponding to the working directory from the
            // archive.
            let cwd = ctx.path.clone();
            cwd.lookup(&mut ctx.decoder)?
        };

        ctx.decoder
            .restore(&start_dir, &Path::new(&target), &match_pattern)?;
        Ok(())
    })
}

#[api(
    input: {
        properties: {
            path: {
                type: String,
                description: "Path to node from where to start the search."
            },
            pattern: {
                type: String,
                description: "Match pattern for matching files in the catalog."
            },
            select: {
                type: bool,
                optional: true,
                description: "Add matching filenames to list for restore."
            }
        }
    }
)]
/// Find entries in the catalog matching the given match pattern.
fn find_command(path: String, pattern: String, select: Option<bool>) -> Result<(), Error> {
    Context::with(|ctx| {
        let mut local = ctx.path.clone();
        local.traverse(&PathBuf::from(path), &mut ctx.decoder, &mut ctx.catalog, false)?;
        let canonical = local.canonical(&mut ctx.decoder, &mut ctx.catalog, false)?;
        if !local.last().is_directory() {
            bail!("path should be a directory, not a file!");
        }
        let select = select.unwrap_or(false);

        let cpath = canonical.generate_cstring().unwrap();
        let pattern = if pattern.starts_with("!") {
            let mut buffer = vec![b'!'];
            buffer.extend_from_slice(cpath.as_bytes());
            buffer.extend_from_slice(pattern[1..pattern.len()].as_bytes());
            buffer
        } else {
            let mut buffer = cpath.as_bytes().to_vec();
            buffer.extend_from_slice(pattern.as_bytes());
            buffer
        };

        let pattern = MatchPattern::from_line(&pattern)?
            .ok_or_else(|| format_err!("invalid match pattern"))?;
        let slice = vec![pattern.as_slice()];

        // The match pattern all contain the prefix of the entry path in order to
        // store them if selected, so the entry point for find is always the root
        // directory.
        let mut dir_stack = vec![ctx.path.root()];
        ctx.catalog.find(
            &mut dir_stack,
            &slice,
            &Box::new(|path: &[DirEntry]| println!("{:?}", Context::generate_cstring(path).unwrap()))
        )?;

        // Insert if matches should be selected.
        // Avoid duplicate entries of the same match pattern.
        if select && ctx.selected.iter().find(|p| **p == pattern).is_none() {
            ctx.selected.push(pattern);
        }

        Ok(())
    })
}

std::thread_local! {
    static CONTEXT: RefCell<Option<Context>> = RefCell::new(None);
}

/// Holds the context needed for access to catalog and decoder
struct Context {
    /// Calalog reader instance to navigate
    catalog: CatalogReader<std::fs::File>,
    /// List of selected paths for restore
    selected: Vec<MatchPattern>,
    /// Decoder instance for the current pxar archive
    decoder: Decoder,
    /// Handle catalog stuff
    path: CatalogPathStack,
}

impl Context {
    /// Execute `call` within a context providing a mut ref to `Context` instance.
    fn with<T, F>(call: F) -> Result<T, Error>
    where
        F: FnOnce(&mut Context) -> Result<T, Error>,
    {
        CONTEXT.with(|cell| {
            let mut ctx = cell.borrow_mut();
            call(&mut ctx.as_mut().unwrap())
        })
    }

    /// Generate CString from provided stack of `DirEntry`s.
    fn generate_cstring(dir_stack: &[DirEntry]) -> Result<CString, Error> {
        let mut path = vec![b'/'];
        // Skip the archive root, the '/' is displayed for it instead
        for component in dir_stack.iter().skip(1) {
            path.extend_from_slice(&component.name);
            if component.is_directory() {
                path.push(b'/');
            }
        }
        Ok(unsafe { CString::from_vec_unchecked(path) })
    }

    /// Generate the CString to display by readline based on
    /// PROMPT_PREFIX, PROMPT and the current working directory.
    fn generate_prompt(&self) -> Result<String, Error> {
        let prompt = format!(
            "{}{} {} ",
            PROMPT_PREFIX,
            self.path.generate_cstring()?.to_string_lossy(),
            PROMPT,
        );
        Ok(prompt)
    }
}

/// A valid path in the catalog starting from root.
///
/// Symlinks are stored by pushing the symlink entry and the target entry onto
/// the stack. Allows to resolve all symlink in order to generate a canonical
/// path needed for reading from the archive.
#[derive(Clone)]
struct CatalogPathStack {
    stack: Vec<DirEntry>,
    root: DirEntry,
}

impl CatalogPathStack {
    /// Create a new stack with given root entry.
    fn new(root: DirEntry) -> Self {
        Self {
            stack: Vec::new(),
            root,
        }
    }

    /// Get a clone of the root directories entry.
    fn root(&self) -> DirEntry {
        self.root.clone()
    }

    /// Remove all entries from the stack.
    ///
    /// This equals to being at the root directory.
    fn clear(&mut self) {
        self.stack.clear();
    }

    /// Get a reference to the last entry on the stack.
    fn last(&self) -> &DirEntry {
        self.stack.last().unwrap_or(&self.root)
    }

    /// Check if the last entry is a symlink.
    fn last_is_symlink(&self) -> bool {
        self.last().is_symlink()
    }

    /// Check if the last entry is a directory.
    fn last_is_directory(&self) -> bool {
        self.last().is_directory()
    }

    /// Remove a component, if it was a symlink target,
    /// this removes also the symlink entry.
    fn pop(&mut self) -> Option<DirEntry> {
        let entry = self.stack.pop()?;
        if self.last_is_symlink() {
            self.stack.pop()
        } else {
            Some(entry)
        }
    }

    /// Add a component to the stack.
    fn push(&mut self, entry: DirEntry) {
        self.stack.push(entry)
    }

    /// Check if pushing the given entry onto the CatalogPathStack would create a
    /// loop by checking if the same entry is already present.
    fn creates_loop(&self, entry: &DirEntry) -> bool {
        self.stack.iter().any(|comp| comp.eq(entry))
    }

    /// Starting from this path, traverse the catalog by the provided `path`.
    fn traverse(
        &mut self,
        path: &PathBuf,
        mut decoder: &mut Decoder,
        mut catalog: &mut CatalogReader<std::fs::File>,
        follow_final: bool,
    ) -> Result<(), Error> {
        for component in path.components() {
            match component {
                Component::RootDir => self.clear(),
                Component::CurDir => continue,
                Component::ParentDir => { self.pop(); }
                Component::Normal(comp) => {
                    let entry = catalog.lookup(self.last(), comp.as_bytes())?;
                    if self.creates_loop(&entry) {
                        bail!("loop detected, will not follow");
                    }
                    self.push(entry);
                    if self.last_is_symlink() && follow_final {
                        let mut canonical = self.canonical(&mut decoder, &mut catalog, follow_final)?;
                        let target = canonical.pop().unwrap();
                        self.push(target);
                    }
                }
                Component::Prefix(_) => bail!("encountered prefix component. Non unix systems not supported."),
            }
        }
        if path.as_os_str().as_bytes().ends_with(b"/") && !self.last_is_directory() {
            bail!("entry is not a directory");
        }
        Ok(())
    }

    /// Create a canonical version of this path with symlinks resolved.
    ///
    /// If resolve final is true, follow also an eventual symlink of the last
    /// path component.
    fn canonical(
        &self,
        mut decoder: &mut Decoder,
        mut catalog: &mut CatalogReader<std::fs::File>,
        resolve_final: bool,
    ) -> Result<Self, Error> {
        let mut canonical = CatalogPathStack::new(self.root.clone());
        let mut iter = self.stack.iter().enumerate();
        while let Some((index, component)) = iter.next() {
            if component.is_directory() {
                canonical.push(component.clone());
            } else if component.is_symlink() {
                canonical.push(component.clone());
                 if index != self.stack.len() - 1 || resolve_final {
                    // Get the symlink target by traversing the canonical path
                    // in the archive up to the symlink.
                    let archive_entry = canonical.lookup(&mut decoder)?;
                    canonical.pop();
                    // Resolving target means also ignoring the target in the iterator, so get it.
                    iter.next();
                    let target = archive_entry.target
                        .ok_or_else(|| format_err!("expected entry with symlink target."))?;
                    canonical.traverse(&target, &mut decoder, &mut catalog, resolve_final)?;
                }
            } else if index != self.stack.len() - 1 {
                bail!("intermitten node is not symlink nor directory");
            } else {
                canonical.push(component.clone());
            }
        }
        Ok(canonical)
    }

    /// Lookup this path in the archive using the provided decoder.
    fn lookup(&self, decoder: &mut Decoder) -> Result<DirectoryEntry, Error> {
        let mut current = decoder.root()?;
        for component in self.stack.iter() {
            match decoder.lookup(&current, &OsStr::from_bytes(&component.name))? {
                Some(item) => current = item,
                // This should not happen if catalog an archive are consistent.
                None => bail!("no such file or directory in archive - inconsistent catalog"),
            }
        }
        Ok(current)
    }

    /// Generate a CString from this.
    fn generate_cstring(&self) -> Result<CString, Error> {
        let mut path = vec![b'/'];
        let mut iter = self.stack.iter().enumerate();
        while let Some((index, component)) = iter.next() {
            if component.is_symlink() && index != self.stack.len() - 1 {
                let (_, next) = iter.next()
                    .ok_or_else(|| format_err!("unresolved symlink encountered"))?;
                // Display the name of the link, not the target
                path.extend_from_slice(&component.name);
                if next.is_directory() {
                    path.push(b'/');
                }
            } else {
                path.extend_from_slice(&component.name);
                if component.is_directory() {
                    path.push(b'/');
                }
            }
        }
        Ok(unsafe { CString::from_vec_unchecked(path) })
    }
}
