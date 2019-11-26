use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString, OsStr};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use failure::*;
use libc;

use crate::pxar::*;

use super::catalog::{CatalogReader, DirEntry};
use super::readline::{Context, Readline};

const PROMPT_PREFIX: &str = "pxar:";
const PROMPT_POST: &str = " > ";

/// Interactive shell for interacton with the catalog.
pub struct Shell {
    /// Actual shell instance with context.
    sh: ShellInstance,
    /// Map containing all the defined commands.
    cmds: ShellCmdMap,
}

impl Shell {
    /// Create a new shell for the given catalog and pxar archive.
    pub fn new(
        catalog: CatalogReader<std::fs::File>,
        archive_name: &str,
        decoder: Decoder,
    ) -> Result<Self, Error> {
        const OPTIONAL: bool = true;
        const REQUIRED: bool = false;

        Ok(Self {
            sh: ShellInstance::new(catalog, archive_name, decoder)?,
            // This list defines all the commands for the shell including their
            // parameters, options and help description.
            cmds: ShellCmdMap::new()
                .insert(ShellCmd::new(
                    "pwd",
                    "List the current working directory.",
                    ShellInstance::pwd,
                ))
                .insert(ShellCmd::new(
                    "ls",
                    "List contents of directory.",
                    ShellInstance::list,
                ).parameter("path", OPTIONAL))
                .insert(ShellCmd::new(
                    "cd",
                    "Change current working directory.",
                    ShellInstance::change_dir,
                ).parameter("path", OPTIONAL))
                .insert(ShellCmd::new(
                    "stat",
                    "Show the status of a file or directory.",
                    ShellInstance::stat,
                ).parameter("path", REQUIRED))
                .insert(ShellCmd::new(
                    "restore",
                    "Restore archive to target (restores only matching entries if match-pattern is provided)",
                    ShellInstance::restore,
                ).option("match", Some("match-pattern")).parameter("target", REQUIRED))
                .insert(ShellCmd::new(
                        "select",
                        "Add a file/directory to the list of entries selected for restore.",
                        ShellInstance::select,
                ).parameter("path", REQUIRED))
                .insert(ShellCmd::new(
                    "selected",
                    "Show the list of entries currently selected for restore.",
                    ShellInstance::list_selected,
                ))
                .insert(ShellCmd::new(
                    "deselect",
                    "Remove a file/directory from the list of entries selected for restore.",
                    ShellInstance::deselect,
                ).parameter("path", REQUIRED))
                .insert(ShellCmd::new(
                    "restore-selected",
                    "Restore the file/directory on the list of entries selected for restore.",
                    ShellInstance::restore_selected,
                ).parameter("target", REQUIRED))
                .insert(ShellCmd::new(
                    "help",
                    "Show all commands or the help for the provided command",
                    ShellInstance::help,
                ).parameter("command", OPTIONAL))
        })
    }

    /// Start the interactive shell loop
    pub fn shell(mut self) -> Result<(), Error> {
        while let Some(line) = self.sh.rl.readline() {
            let (cmd, args) = match self.cmds.parse(&line) {
                Ok(res) => res,
                Err(err) => {
                    println!("error: {}", err);
                    continue;
                }
            };
            // Help is treated a bit separate as we need the full command list,
            // which would not be accessible in the callback.
            if cmd.command == "help" {
                match args.get_param("command") {
                    Some(name) => match self.cmds.cmds.get(name) {
                        Some(cmd) => println!("{}", cmd.help()),
                        None => println!("no help for command"),
                    },
                    None => self.cmds.list_commands(),
                }
            }
            match (cmd.callback)(&mut self.sh, args) {
                Ok(_) => (),
                Err(err) => {
                    println!("error: {}", err);
                    continue;
                }
            };
        }
        Ok(())
    }
}

/// Stores the command definitions for the known commands.
struct ShellCmdMap {
    cmds: HashMap<&'static [u8], ShellCmd>,
}

impl ShellCmdMap {
    fn new() -> Self {
        Self {
            cmds: HashMap::new(),
        }
    }

    /// Insert a new `ShellCmd` into the `ShellCmdMap`
    fn insert(mut self, cmd: ShellCmd) -> Self {
        self.cmds.insert(cmd.command.as_bytes(), cmd);
        self
    }

    /// List all known commands with their help text.
    fn list_commands(&self) {
        println!();
        for cmd in &self.cmds {
            println!("{}\n", cmd.1.help());
        }
    }

    /// Parse the given line and interprete it based on the known commands in
    /// this `ShellCmdMap` instance.
    fn parse<'a>(&'a self, line: &'a [u8]) -> Result<(&'a ShellCmd, Args), Error> {
        // readline already handles tabs, so here we only split on spaces
        let args: Vec<&[u8]> = line
            .split(|b| *b == b' ')
            .filter(|word| !word.is_empty())
            .collect();
        let mut args = args.iter();
        let arg0 = args
            .next()
            .ok_or_else(|| format_err!("no command provided"))?;
        let cmd = self
            .cmds
            .get(arg0)
            .ok_or_else(|| format_err!("invalid command"))?;
        let mut given = Args {
            options: HashMap::new(),
            parameters: HashMap::new(),
        };
        let mut required = cmd.required_parameters.iter();
        let mut optional = cmd.optional_parameters.iter();
        while let Some(arg) = args.next() {
            if arg.starts_with(b"--") {
                let opt = cmd
                    .options
                    .iter()
                    .find(|opt| opt.0.as_bytes() == &arg[2..arg.len()]);
                if let Some(opt) = opt {
                    if opt.1.is_some() {
                        // Expect a parameter for the given option
                        let opt_param = args.next().ok_or_else(|| {
                            format_err!("expected parameter for option {}", opt.0)
                        })?;
                        given.options.insert(opt.0, Some(opt_param));
                    } else {
                        given.options.insert(opt.0, None);
                    }
                } else {
                    bail!("invalid option");
                }
            } else if let Some(name) = required.next() {
                // First fill all required parameters
                given.parameters.insert(name, arg);
            } else if let Some(name) = optional.next() {
                // Now fill all optional parameters
                given.parameters.insert(name, arg);
            } else {
                bail!("to many arguments");
            }
        }
        // Check that we have got all required parameters
        if required.next().is_some() {
            bail!("not all required parameters provided");
        }
        Ok((cmd, given))
    }
}

/// Interpreted CLI arguments, stores parameters and options.
struct Args<'a> {
    parameters: HashMap<&'static str, &'a [u8]>,
    options: HashMap<&'static str, Option<&'a [u8]>>,
}

impl<'a> Args<'a> {
    /// Get a reference to the parameter give by name if present
    fn get_param(&self, name: &str) -> Option<&&'a [u8]> {
        self.parameters.get(name)
    }

    /// Get a reference to the option give by name if present
    fn get_opt(&self, name: &str) -> Option<&Option<&'a [u8]>> {
        self.options.get(name)
    }
}

/// Definition of a shell command with its name, callback, description and
/// argument definition.
struct ShellCmd {
    command: &'static str,
    callback: fn(&mut ShellInstance, Args) -> Result<(), Error>,
    description: &'static str,
    options: Vec<(&'static str, Option<&'static str>)>,
    required_parameters: Vec<&'static str>,
    optional_parameters: Vec<&'static str>,
}

impl ShellCmd {
    /// Define a new `ShellCmd` with given command name, description and callback function.
    fn new(
        command: &'static str,
        description: &'static str,
        callback: fn(&mut ShellInstance, Args) -> Result<(), Error>,
    ) -> Self {
        Self {
            command,
            callback,
            description,
            options: Vec::new(),
            required_parameters: Vec::new(),
            optional_parameters: Vec::new(),
        }
    }

    /// Add additional named parameter `parameter` to command definition.
    ///
    /// The optional flag indicates if this parameter is required or optional.
    fn parameter(mut self, parameter: &'static str, optional: bool) -> Self {
        if optional {
            self.optional_parameters.push(parameter);
        } else {
            self.required_parameters.push(parameter);
        }
        self
    }

    /// Add additional named option `option` to command definition.
    ///
    /// The Option `parameter` indicates if this option has an additional parameter or not.
    fn option(mut self, option: &'static str, parameter: Option<&'static str>) -> Self {
        self.options.push((option, parameter));
        self
    }

    /// Create the help String for this command
    fn help(&self) -> String {
        let mut help = String::new();
        help.push_str(self.command);
        help.push_str("\n  Usage:\t");
        help.push_str(self.command);
        for opt in &self.options {
            help.push_str(" [--");
            help.push_str(opt.0);
            if let Some(opt_param) = opt.1 {
                help.push(' ');
                help.push_str(opt_param);
            }
            help.push(']');
        }
        for par in &self.required_parameters {
            help.push(' ');
            help.push_str(par);
        }
        for par in &self.optional_parameters {
            help.push_str(" [");
            help.push_str(par);
            help.push(']');
        }
        help.push_str("\n  Description:\t");
        help.push_str(self.description);
        help
    }
}

/// State of the shell instance
struct ShellInstance {
    /// Readline context
    rl: Readline,
    /// List of paths selected for a restore
    selected: HashSet<Vec<u8>>,
    /// Decoder instance for the current pxar archive
    decoder: Decoder,
    /// Root directory for the give archive as stored in the catalog
    root: Vec<DirEntry>,
}

impl ShellInstance {
    /// Create a new `ShellInstance` for the given catalog and archive.
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

    /// Generate the CString to display by readline based on the
    /// PROMPT_PREFIX, PROMPT_POST and the given byte slice.
    fn generate_prompt(path: &[u8]) -> CString {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(PROMPT_PREFIX.as_bytes());
        buffer.extend_from_slice(path);
        buffer.extend_from_slice(PROMPT_POST.as_bytes());
        unsafe { CString::from_vec_unchecked(buffer) }
    }

    /// Get a mut ref to the context in order to be able to access the
    /// catalog and the directory stack for the current working directory.
    fn context(&mut self) -> &mut Context {
        self.rl.context()
    }

    /// Change the current working directory to the new directory
    fn change_dir(&mut self, args: Args) -> Result<(), Error> {
        let path = match args.get_param("path") {
            Some(path) => *path,
            None => &[],
        };
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
        let prompt =
            Self::generate_prompt(Self::path(&self.context().current.clone())?.as_slice());
        self.rl.update_prompt(prompt);
        Ok(())
    }

    /// List the content of a directory.
    ///
    /// Executed on files it returns the DirEntry of the file as single element
    /// in the list.
    fn list(&mut self, args: Args) -> Result<(), Error> {
        let parent = if let Some(path) = args.get_param("path") {
            // !path.is_empty() {
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
        Self::print_list(&list).map_err(|err| format_err!("{}", err))
    }

    /// Print the current working directory
    fn pwd(&mut self, _args: Args) -> Result<(), Error> {
        let pwd = Self::path(&self.context().current.clone())?;
        Self::print_slice(&pwd).map_err(|err| format_err!("{}", err))
    }

    /// Generate an absolute path from a directory stack.
    fn path(dir_stack: &[DirEntry]) -> Result<Vec<u8>, Error> {
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
                    let entry = self
                        .context()
                        .catalog
                        .lookup(dir_stack.last().unwrap(), name)?;
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

    /// Read the metadata for a given directory entry.
    ///
    /// This is expensive because the data has to be read from the pxar `Decoder`,
    /// which means reading over the network.
    fn stat(&mut self, args: Args) -> Result<(), Error> {
        let path = args
            .get_param("path")
            .ok_or_else(|| format_err!("no path provided"))?;
        // First check if the file exists in the catalog, therefore avoiding
        // expensive calls to the decoder just to find out that there could be no
        // such entry. This is done by calling canonical_path(), which returns
        // the full path if it exists, error otherwise.
        let path = self.canonical_path(path)?;
        let (entry, attr, size) = self.lookup(&path)?;
        Self::print_stat(&entry, &attr, size).map_err(|err| format_err!("{}", err))
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
            match self
                .decoder
                .lookup(&current, &OsStr::from_bytes(&item.name))?
            {
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
    fn select(&mut self, args: Args) -> Result<(), Error> {
        let path = args
            .get_param("path")
            .ok_or_else(|| format_err!("no path provided"))?;
        // Calling canonical_path() makes sure the provided path is valid and
        // actually contained within the catalog and therefore also the archive.
        let path = self.canonical_path(path)?;
        if self.selected.insert(Self::path(&path)?) {
            Ok(())
        } else {
            bail!("entry already selected for restore")
        }
    }

    /// Deselect an entry for restore.
    ///
    /// This will return an error if the entry was not found in the list of entries
    /// selected for restore.
    fn deselect(&mut self, args: Args) -> Result<(), Error> {
        let path = args
            .get_param("path")
            .ok_or_else(|| format_err!("no path provided"))?;
        if self.selected.remove(*path) {
            Ok(())
        } else {
            bail!("entry not selected for restore")
        }
    }

    /// Restore the selected entries to the given target path.
    ///
    /// Target must not exist on the clients filesystem.
    fn restore_selected(&mut self, args: Args) -> Result<(), Error> {
        let target = args
            .get_param("target")
            .ok_or_else(|| format_err!("no target provided"))?;
        let mut list = Vec::new();
        for path in &self.selected {
            let pattern = MatchPattern::from_line(path)?
                .ok_or_else(|| format_err!("encountered invalid match pattern"))?;
            list.push(pattern);
        }
        if list.is_empty() {
            bail!("no entries selected for restore");
        }

        // Entry point for the restore is always root here as the provided match
        // patterns are relative to root as well.
        let start_dir = self.decoder.root()?;
        let target: &OsStr = OsStrExt::from_bytes(target);
        self.decoder
            .restore(&start_dir, &Path::new(target), &list)?;
        Ok(())
    }

    /// List entries currently selected for restore.
    fn list_selected(&mut self, _args: Args) -> Result<(), Error> {
        let mut out = std::io::stdout();
        for entry in &self.selected {
            out.write_all(entry).map_err(|err| format_err!("{}", err))?;
            out.write_all(&[b'\n'])
                .map_err(|err| format_err!("{}", err))?;
        }
        out.flush().map_err(|err| format_err!("{}", err))?;
        Ok(())
    }

    /// Restore the sub-archive given by the current working directory to target.
    ///
    /// By further providing a pattern, the restore can be limited to a narrower
    /// subset of this sub-archive.
    /// If pattern is an empty slice, the full dir is restored.
    fn restore(&mut self, args: Args) -> Result<(), Error> {
        let target = args.get_param("target").unwrap();
        let pattern = args.get_opt("pattern").unwrap().unwrap_or(&[]);
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
        self.decoder
            .restore(&start_dir, &Path::new(target), &match_pattern)?;
        Ok(())
    }

    /// Dummy callback for the help command.
    fn help(&mut self, _args: Args) -> Result<(), Error> {
        // this is a dummy, the actual help is handled before calling the callback
        // as the full set of available commands is needed.
        Ok(())
    }

    /// Print the list of `DirEntry`s to stdout.
    fn print_list(list: &[DirEntry]) -> Result<(), std::io::Error> {
        if list.is_empty() {
            return Ok(());
        }
        let max = list.iter().max_by(|x, y| x.name.len().cmp(&y.name.len()));
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
    }

    /// Print the given byte slice to stdout.
    fn print_slice(slice: &[u8]) -> Result<(), std::io::Error> {
        let mut out = std::io::stdout();
        out.write_all(slice)?;
        out.write_all(&[b'\n'])?;
        out.flush()?;
        Ok(())
    }

    /// Print the stats of `DirEntry` item to stdout.
    fn print_stat(
        item: &DirectoryEntry,
        _attr: &PxarAttributes,
        size: u64,
    ) -> Result<(), std::io::Error> {
        let mut out = std::io::stdout();
        out.write_all(b"File: ")?;
        out.write_all(&item.filename.as_bytes())?;
        out.write_all(&[b'\n'])?;
        out.write_all(format!("Size: {}\n", size).as_bytes())?;
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
}

/// Filename completion callback for the shell
// TODO: impl command completion. For now only filename completion.
fn complete(ctx: &mut Context, text: &CStr, _start: usize, _end: usize) -> Vec<CString> {
    let slices: Vec<_> = text.to_bytes().split(|b| *b == b'/').collect();
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
                    if current.len() > 1 {
                        current.pop();
                    }
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
