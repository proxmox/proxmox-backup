use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CString, OsStr};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use failure::*;

use super::catalog::{CatalogReader, DirEntry};
use crate::pxar::*;
use crate::tools;

use proxmox::api::{cli::*, *};

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
        let root = vec![archive_root];

        CONTEXT.with(|handle| {
            let mut ctx = handle.borrow_mut();
            *ctx = Some(Context {
                catalog,
                selected: Vec::new(),
                decoder,
                root: root.clone(),
                current: root,
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
            let _ = handle_command(helper.cmd_def(), "", args);
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
                ctx.current.clone()
            } else {
                ctx.canonical_path(base)?
            };

            let entries = match ctx.catalog.read_dir(&current.last().unwrap()) {
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
        let path = Context::generate_cstring(&ctx.current)?;
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
        let mut path = ctx.canonical_path(&path)?;
        if !path
            .last()
            .ok_or_else(|| format_err!("invalid path component"))?
            .is_directory()
        {
            // Change to the parent dir of the file instead
            path.pop();
            eprintln!("not a directory, fallback to parent directory");
        }
        ctx.current = path;
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
        let parent = if let Some(path) = path {
            ctx.canonical_path(&path)?
                .last()
                .ok_or_else(|| format_err!("invalid path component"))?
                .clone()
        } else {
            ctx.current.last().unwrap().clone()
        };

        let list = if parent.is_directory() {
            ctx.catalog.read_dir(&parent)?
        } else {
            vec![parent]
        };

        if list.is_empty() {
            return Ok(());
        }
        let max = list.iter().max_by(|x, y| x.name.len().cmp(&y.name.len()));
        let max = match max {
            Some(dir_entry) => dir_entry.name.len() + 1,
            None => 0,
        };

        let (_rows, mut cols) = Context::get_terminal_size();
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
        // First check if the file exists in the catalog, therefore avoiding
        // expensive calls to the decoder just to find out that there maybe is
        // no such entry.
        // This is done by calling canonical_path(), which returns the full path
        // if it exists, error otherwise.
        let path = ctx.canonical_path(&path)?;
        let item = ctx.lookup(&path)?;
        let mut out = std::io::stdout();
        out.write_all(b"File: ")?;
        out.write_all(item.filename.as_bytes())?;
        out.write_all(&[b'\n'])?;
        out.write_all(format!("Size: {}\n", item.size).as_bytes())?;
        out.write_all(b"Type: ")?;
        match item.entry.mode as u32 & libc::S_IFMT {
            libc::S_IFDIR => out.write_all(b"directory\n")?,
            libc::S_IFREG => out.write_all(b"regular file\n")?,
            libc::S_IFLNK => out.write_all(b"symbolic link\n")?,
            libc::S_IFBLK => out.write_all(b"block special file\n")?,
            libc::S_IFCHR => out.write_all(b"character special file\n")?,
            _ => out.write_all(b"unknown\n")?,
        };
        out.write_all(format!("Uid: {}\n", item.entry.uid).as_bytes())?;
        out.write_all(format!("Gid: {}\n", item.entry.gid).as_bytes())?;
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
        // Calling canonical_path() makes sure the provided path is valid and
        // actually contained within the catalog and therefore also the archive.
        let path = ctx.canonical_path(&path)?;
        let pattern = MatchPattern::from_line(Context::generate_cstring(&path)?.as_bytes())?
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
        let path = ctx.canonical_path(&path)?;
        let mut pattern = MatchPattern::from_line(Context::generate_cstring(&path)?.as_bytes())?
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

#[api( input: { properties: {} })]
/// List entries currently selected for restore.
fn list_selected_command() -> Result<(), Error> {
    Context::with(|ctx| {
        let mut out = std::io::stdout();
        out.write_all(&MatchPattern::to_bytes(ctx.selected.as_slice()))?;
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
            let cwd = ctx.current.clone();
            ctx.lookup(&cwd)?
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
        let path = ctx.canonical_path(&path)?;
        if !path.last().unwrap().is_directory() {
            bail!("path should be a directory, not a file!");
        }
        let select = select.unwrap_or(false);

        let cpath = Context::generate_cstring(&path).unwrap();
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
        let mut dir_stack = ctx.root.clone();
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
    /// Root directory for the give archive as stored in the catalog
    root: Vec<DirEntry>,
    /// Stack of directories up to the current working directory
    /// used for navigation and path completion.
    current: Vec<DirEntry>,
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

    /// Resolve the indirect path components and return an absolute path.
    ///
    /// This will actually navigate the filesystem tree to check that the
    /// path is vaild and exists.
    /// This does not include following symbolic links.
    /// If None is given as path, only the root directory is returned.
    fn canonical_path(&mut self, path: &str) -> Result<Vec<DirEntry>, Error> {
        if path == "/" {
            return Ok(self.root.clone());
        }

        let mut path_slice = if path.is_empty() {
            // Fallback to root if no path was provided
            return Ok(self.root.clone());
        } else {
            path
        };

        let mut dir_stack = if path_slice.starts_with("/") {
            // Absolute path, reduce view of slice and start from root
            path_slice = &path_slice[1..];
            self.root.clone()
        } else {
            // Relative path, start from current working directory
            self.current.clone()
        };
        let should_end_dir = if path_slice.ends_with("/") {
            path_slice = &path_slice[0..path_slice.len() - 1];
            true
        } else {
            false
        };
        for name in path_slice.split('/') {
            match name {
                "." => continue,
                ".." => {
                    // Never pop archive root from stack
                    if dir_stack.len() > 1 {
                        dir_stack.pop();
                    }
                }
                _ => {
                    let entry = self.catalog.lookup(dir_stack.last().unwrap(), name.as_bytes())?;
                    dir_stack.push(entry);
                }
            }
        }
        if should_end_dir
            && !dir_stack
                .last()
                .ok_or_else(|| format_err!("invalid path component"))?
                .is_directory()
        {
            bail!("entry is not a directory");
        }

        Ok(dir_stack)
    }

    /// Generate the CString to display by readline based on
    /// PROMPT_PREFIX, PROMPT and the current working directory.
    fn generate_prompt(&self) -> Result<String, Error> {
        let prompt = format!(
            "{}{} {} ",
            PROMPT_PREFIX,
            Self::generate_cstring(&self.current)?.to_string_lossy(),
            PROMPT,
        );
        Ok(prompt)
    }

    /// Get the current size of the terminal
    /// # Safety
    ///
    /// uses unsafe call to tty_ioctl, see man tty_ioctl(2)
    fn get_terminal_size() -> (usize, usize) {
        const TIOCGWINSZ: libc::c_ulong = 0x5413;

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

    /// Look up the entry given by a canonical absolute `path` in the archive.
    ///
    /// This will actively navigate the archive by calling the corresponding
    /// decoder functionalities and is therefore very expensive.
    fn lookup(&mut self, absolute_path: &[DirEntry]) -> Result<DirectoryEntry, Error> {
        let mut current = self.decoder.root()?;
        // Ignore the archive root, don't need it.
        for item in absolute_path.iter().skip(1) {
            match self
                .decoder
                .lookup(&current, &OsStr::from_bytes(&item.name))?
            {
                Some(item) => current = item,
                // This should not happen if catalog an archive are consistent.
                None => bail!("no such file or directory in archive - inconsistent catalog"),
            }
        }
        Ok(current)
    }
}
