use std::collections::HashMap;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::future::Future;
use std::io::Write;
use std::mem;
use std::ops::ControlFlow;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::pin::Pin;

use anyhow::{bail, format_err, Error};
use nix::dir::Dir;
use nix::fcntl::OFlag;
use nix::sys::stat::Mode;

use pathpatterns::{MatchEntry, MatchList, MatchPattern, MatchType, PatternFlag};
use proxmox_router::cli::{self, CliCommand, CliCommandMap, CliHelper, CommandLineInterface};
use proxmox_schema::api;
use proxmox_sys::fs::{create_path, CreateOptions};
use pxar::accessor::ReadAt;
use pxar::{EntryKind, Metadata};

use pbs_datastore::catalog::{self, DirEntryAttribute};
use proxmox_async::runtime::block_in_place;

use crate::pxar::Flags;

type CatalogReader = pbs_datastore::catalog::CatalogReader<std::fs::File>;

type Reader = std::sync::Arc<dyn ReadAt + Send + Sync + 'static>;
type Accessor = pxar::accessor::aio::Accessor<Reader>;
type FileEntry = pxar::accessor::aio::FileEntry<Reader>;

const MAX_SYMLINK_COUNT: usize = 40;

static mut SHELL: Option<usize> = None;

/// This list defines all the shell commands and their properties
/// using the api schema
pub fn catalog_shell_cli() -> CommandLineInterface {
    CommandLineInterface::Nested(
        CliCommandMap::new()
            .insert("pwd", CliCommand::new(&API_METHOD_PWD_COMMAND))
            .insert(
                "cd",
                CliCommand::new(&API_METHOD_CD_COMMAND)
                    .arg_param(&["path"])
                    .completion_cb("path", complete_path),
            )
            .insert(
                "ls",
                CliCommand::new(&API_METHOD_LS_COMMAND)
                    .arg_param(&["path"])
                    .completion_cb("path", complete_path),
            )
            .insert(
                "stat",
                CliCommand::new(&API_METHOD_STAT_COMMAND)
                    .arg_param(&["path"])
                    .completion_cb("path", complete_path),
            )
            .insert(
                "select",
                CliCommand::new(&API_METHOD_SELECT_COMMAND)
                    .arg_param(&["path"])
                    .completion_cb("path", complete_path),
            )
            .insert(
                "deselect",
                CliCommand::new(&API_METHOD_DESELECT_COMMAND)
                    .arg_param(&["path"])
                    .completion_cb("path", complete_path),
            )
            .insert(
                "clear-selected",
                CliCommand::new(&API_METHOD_CLEAR_SELECTED_COMMAND),
            )
            .insert(
                "list-selected",
                CliCommand::new(&API_METHOD_LIST_SELECTED_COMMAND),
            )
            .insert(
                "restore-selected",
                CliCommand::new(&API_METHOD_RESTORE_SELECTED_COMMAND)
                    .arg_param(&["target"])
                    .completion_cb("target", cli::complete_file_name),
            )
            .insert(
                "restore",
                CliCommand::new(&API_METHOD_RESTORE_COMMAND)
                    .arg_param(&["target"])
                    .completion_cb("target", cli::complete_file_name),
            )
            .insert(
                "find",
                CliCommand::new(&API_METHOD_FIND_COMMAND).arg_param(&["pattern"]),
            )
            .insert("exit", CliCommand::new(&API_METHOD_EXIT))
            .insert_help(),
    )
}

fn complete_path(complete_me: &str, _map: &HashMap<String, String>) -> Vec<String> {
    let shell: &mut Shell = unsafe { std::mem::transmute(SHELL.unwrap()) };
    match shell.complete_path(complete_me) {
        Ok(list) => list,
        Err(err) => {
            log::error!("error during completion: {}", err);
            Vec::new()
        }
    }
}

// just an empty wrapper so that it is displayed in help/docs, we check
// in the readloop for 'exit' again break
#[api(input: { properties: {} })]
/// Exit the shell
async fn exit() -> Result<(), Error> {
    Ok(())
}

#[api(input: { properties: {} })]
/// List the current working directory.
async fn pwd_command() -> Result<(), Error> {
    Shell::with(move |shell| shell.pwd()).await
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
async fn cd_command(path: Option<String>) -> Result<(), Error> {
    let path = path.as_ref().map(Path::new);
    Shell::with(move |shell| shell.cd(path)).await
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
async fn ls_command(path: Option<String>) -> Result<(), Error> {
    let path = path.as_ref().map(Path::new);
    Shell::with(move |shell| shell.ls(path)).await
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
/// This is expensive because the data has to be read from the pxar archive, which means reading
/// over the network.
async fn stat_command(path: String) -> Result<(), Error> {
    Shell::with(move |shell| shell.stat(PathBuf::from(path))).await
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
async fn select_command(path: String) -> Result<(), Error> {
    Shell::with(move |shell| shell.select(PathBuf::from(path))).await
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
async fn deselect_command(path: String) -> Result<(), Error> {
    Shell::with(move |shell| shell.deselect(PathBuf::from(path))).await
}

#[api( input: { properties: { } })]
/// Clear the list of files selected for restore.
async fn clear_selected_command() -> Result<(), Error> {
    Shell::with(move |shell| shell.deselect_all()).await
}

#[api(
    input: {
        properties: {
            patterns: {
                type: Boolean,
                description: "List match patterns instead of the matching files.",
                optional: true,
                default: false,
            }
        }
    }
)]
/// List entries currently selected for restore.
async fn list_selected_command(patterns: bool) -> Result<(), Error> {
    Shell::with(move |shell| shell.list_selected(patterns)).await
}

#[api(
    input: {
        properties: {
            pattern: {
                type: String,
                description: "Match pattern for matching files in the catalog."
            },
            select: {
                type: bool,
                optional: true,
                default: false,
                description: "Add matching filenames to list for restore."
            }
        }
    }
)]
/// Find entries in the catalog matching the given match pattern.
async fn find_command(pattern: String, select: bool) -> Result<(), Error> {
    Shell::with(move |shell| shell.find(pattern, select)).await
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
async fn restore_selected_command(target: String) -> Result<(), Error> {
    Shell::with(move |shell| shell.restore_selected(PathBuf::from(target))).await
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
async fn restore_command(target: String, pattern: Option<String>) -> Result<(), Error> {
    Shell::with(move |shell| shell.restore(PathBuf::from(target), pattern)).await
}

/// TODO: Should we use this to fix `step()`? Make path resolution behave more like described in
/// the path_resolution(7) man page.
///
/// The `Path` type's component iterator does not tell us anything about trailing slashes or
/// trailing `Component::CurDir` entries. Since we only support regular paths we'll roll our own
/// here:
enum PathComponent<'a> {
    Root,
    CurDir,
    ParentDir,
    Normal(&'a OsStr),
    TrailingSlash,
}

struct PathComponentIter<'a> {
    path: &'a [u8],
    state: u8, // 0=beginning, 1=ongoing, 2=trailing, 3=finished (fused)
}

impl std::iter::FusedIterator for PathComponentIter<'_> {}

impl<'a> Iterator for PathComponentIter<'a> {
    type Item = PathComponent<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.path.is_empty() {
            return None;
        }

        if self.state == 0 {
            self.state = 1;
            if self.path[0] == b'/' {
                // absolute path
                self.path = &self.path[1..];
                return Some(PathComponent::Root);
            }
        }

        // skip slashes
        let had_slashes = self.path[0] == b'/';
        while self.path.first().copied() == Some(b'/') {
            self.path = &self.path[1..];
        }

        Some(match self.path {
            [] if had_slashes => PathComponent::TrailingSlash,
            [] => return None,
            [b'.'] | [b'.', b'/', ..] => {
                self.path = &self.path[1..];
                PathComponent::CurDir
            }
            [b'.', b'.'] | [b'.', b'.', b'/', ..] => {
                self.path = &self.path[2..];
                PathComponent::ParentDir
            }
            _ => {
                let end = self
                    .path
                    .iter()
                    .position(|&b| b == b'/')
                    .unwrap_or(self.path.len());
                let (out, rest) = self.path.split_at(end);
                self.path = rest;
                PathComponent::Normal(OsStr::from_bytes(out))
            }
        })
    }
}

pub struct Shell {
    /// Readline instance handling input and callbacks
    rl: rustyline::Editor<CliHelper>,

    /// Interactive prompt.
    prompt: String,

    /// Catalog reader instance to navigate
    catalog: CatalogReader,

    /// List of selected paths for restore
    selected: HashMap<OsString, MatchEntry>,

    /// pxar accessor instance for the current pxar archive
    accessor: Accessor,

    /// The current position in the archive.
    position: Vec<PathStackEntry>,
}

#[derive(Clone)]
struct PathStackEntry {
    /// This is always available. We mainly navigate through the catalog.
    catalog: catalog::DirEntry,

    /// Whenever we need something from the actual archive we fill this out. This is cached along
    /// the entire path.
    pxar: Option<FileEntry>,
}

impl PathStackEntry {
    fn new(dir_entry: catalog::DirEntry) -> Self {
        Self {
            pxar: None,
            catalog: dir_entry,
        }
    }
}

impl Shell {
    /// Create a new shell for the given catalog and pxar archive.
    pub async fn new(
        mut catalog: CatalogReader,
        archive_name: &str,
        archive: Accessor,
    ) -> Result<Self, Error> {
        let cli_helper = CliHelper::new(catalog_shell_cli());
        let mut rl = rustyline::Editor::<CliHelper>::new();
        rl.set_helper(Some(cli_helper));

        let catalog_root = catalog.root()?;
        let archive_root = catalog
            .lookup(&catalog_root, archive_name.as_bytes())?
            .ok_or_else(|| format_err!("archive not found in catalog"))?;
        let position = vec![PathStackEntry::new(archive_root)];

        let mut this = Self {
            rl,
            prompt: String::new(),
            catalog,
            selected: HashMap::new(),
            accessor: archive,
            position,
        };
        this.update_prompt();
        Ok(this)
    }

    async fn with<'a, Fut, R, F>(call: F) -> Result<R, Error>
    where
        F: FnOnce(&'a mut Shell) -> Fut,
        Fut: Future<Output = Result<R, Error>>,
        F: 'a,
        Fut: 'a,
        R: 'static,
    {
        let shell: &mut Shell = unsafe { std::mem::transmute(SHELL.unwrap()) };
        call(&mut *shell).await
    }

    pub async fn shell(mut self) -> Result<(), Error> {
        let this = &mut self;
        unsafe {
            SHELL = Some(this as *mut Shell as usize);
        }
        while let Ok(line) = this.rl.readline(&this.prompt) {
            if line == "exit" {
                break;
            }
            let helper = this.rl.helper().unwrap();
            let args = match cli::shellword_split(&line) {
                Ok(args) => args,
                Err(err) => {
                    log::error!("Error: {}", err);
                    continue;
                }
            };

            let _ =
                cli::handle_command_future(helper.cmd_def(), "", args, cli::CliEnvironment::new())
                    .await;
            this.rl.add_history_entry(line);
            this.update_prompt();
        }
        Ok(())
    }

    fn update_prompt(&mut self) {
        self.prompt = "pxar:".to_string();
        if self.position.len() <= 1 {
            self.prompt.push('/');
        } else {
            for p in self.position.iter().skip(1) {
                if !p.catalog.name.starts_with(b"/") {
                    self.prompt.push('/');
                }
                match std::str::from_utf8(&p.catalog.name) {
                    Ok(entry) => self.prompt.push_str(entry),
                    Err(_) => self.prompt.push_str("<non-utf8-dir>"),
                }
            }
        }
        self.prompt.push_str(" > ");
    }

    async fn pwd(&mut self) -> Result<(), Error> {
        let stack = Self::lookup(
            &self.position,
            &mut self.catalog,
            &self.accessor,
            None,
            &mut Some(0),
        )
        .await?;
        let path = Self::format_path_stack(&stack);
        println!("{:?}", path);
        Ok(())
    }

    fn new_path_stack(&self) -> Vec<PathStackEntry> {
        self.position[..1].to_vec()
    }

    async fn resolve_symlink(
        stack: &mut Vec<PathStackEntry>,
        catalog: &mut CatalogReader,
        accessor: &Accessor,
        follow_symlinks: &mut Option<usize>,
    ) -> Result<(), Error> {
        if let Some(ref mut symlink_count) = follow_symlinks {
            *symlink_count += 1;
            if *symlink_count > MAX_SYMLINK_COUNT {
                bail!("too many levels of symbolic links");
            }

            let file = Self::walk_pxar_archive(accessor, &mut stack[..]).await?;

            let path = match file.entry().kind() {
                EntryKind::Symlink(symlink) => Path::new(symlink.as_os_str()),
                _ => bail!("symlink in the catalog was not a symlink in the archive"),
            };

            let new_stack =
                Self::lookup(stack, &mut *catalog, accessor, Some(path), follow_symlinks).await?;

            *stack = new_stack;

            Ok(())
        } else {
            bail!("target is a symlink");
        }
    }

    /// Walk a path and add it to the path stack.
    ///
    /// If the symlink count is used, symlinks will be followed, until we hit the cap and error
    /// out.
    async fn step(
        stack: &mut Vec<PathStackEntry>,
        catalog: &mut CatalogReader,
        accessor: &Accessor,
        component: std::path::Component<'_>,
        follow_symlinks: &mut Option<usize>,
    ) -> Result<(), Error> {
        use std::path::Component;
        match component {
            Component::Prefix(_) => bail!("invalid path component (prefix)"),
            Component::RootDir => stack.truncate(1),
            Component::CurDir => {
                if stack.last().unwrap().catalog.is_symlink() {
                    Self::resolve_symlink(stack, catalog, accessor, follow_symlinks).await?;
                }
            }
            Component::ParentDir => drop(stack.pop()),
            Component::Normal(entry) => {
                if stack.last().unwrap().catalog.is_symlink() {
                    Self::resolve_symlink(stack, catalog, accessor, follow_symlinks).await?;
                }
                match catalog.lookup(&stack.last().unwrap().catalog, entry.as_bytes())? {
                    Some(dir) => stack.push(PathStackEntry::new(dir)),
                    None => bail!("no such file or directory: {:?}", entry),
                }
            }
        }

        Ok(())
    }

    fn step_nofollow(
        stack: &mut Vec<PathStackEntry>,
        catalog: &mut CatalogReader,
        component: std::path::Component<'_>,
    ) -> Result<(), Error> {
        use std::path::Component;
        match component {
            Component::Prefix(_) => bail!("invalid path component (prefix)"),
            Component::RootDir => stack.truncate(1),
            Component::CurDir => {
                if stack.last().unwrap().catalog.is_symlink() {
                    bail!("target is a symlink");
                }
            }
            Component::ParentDir => drop(stack.pop()),
            Component::Normal(entry) => {
                if stack.last().unwrap().catalog.is_symlink() {
                    bail!("target is a symlink");
                } else {
                    match catalog.lookup(&stack.last().unwrap().catalog, entry.as_bytes())? {
                        Some(dir) => stack.push(PathStackEntry::new(dir)),
                        None => bail!("no such file or directory: {:?}", entry),
                    }
                }
            }
        }
        Ok(())
    }

    /// The pxar accessor is required to resolve symbolic links
    async fn walk_catalog(
        stack: &mut Vec<PathStackEntry>,
        catalog: &mut CatalogReader,
        accessor: &Accessor,
        path: &Path,
        follow_symlinks: &mut Option<usize>,
    ) -> Result<(), Error> {
        for c in path.components() {
            Self::step(stack, catalog, accessor, c, follow_symlinks).await?;
        }
        Ok(())
    }

    /// Non-async version cannot follow symlinks.
    fn walk_catalog_nofollow(
        stack: &mut Vec<PathStackEntry>,
        catalog: &mut CatalogReader,
        path: &Path,
    ) -> Result<(), Error> {
        for c in path.components() {
            Self::step_nofollow(stack, catalog, c)?;
        }
        Ok(())
    }

    /// This assumes that there are no more symlinks in the path stack.
    async fn walk_pxar_archive(
        accessor: &Accessor,
        stack: &mut [PathStackEntry],
    ) -> Result<FileEntry, Error> {
        if stack[0].pxar.is_none() {
            stack[0].pxar = Some(accessor.open_root().await?.lookup_self().await?);
        }

        // Now walk the directory stack:
        let mut at = 1;
        while at < stack.len() {
            if stack[at].pxar.is_some() {
                at += 1;
                continue;
            }

            let parent = stack[at - 1].pxar.as_ref().unwrap();
            let dir = parent.enter_directory().await?;
            let name = Path::new(OsStr::from_bytes(&stack[at].catalog.name));
            stack[at].pxar = Some(
                dir.lookup(name)
                    .await?
                    .ok_or_else(|| format_err!("no such entry in pxar file: {:?}", name))?,
            );

            at += 1;
        }

        Ok(stack.last().unwrap().pxar.clone().unwrap())
    }

    fn complete_path(&mut self, input: &str) -> Result<Vec<String>, Error> {
        let mut tmp_stack;
        let (parent, base, part) = match input.rfind('/') {
            Some(ind) => {
                let (base, part) = input.split_at(ind + 1);
                let path = PathBuf::from(base);
                if path.is_absolute() {
                    tmp_stack = self.new_path_stack();
                } else {
                    tmp_stack = self.position.clone();
                }
                Self::walk_catalog_nofollow(&mut tmp_stack, &mut self.catalog, &path)?;
                (&tmp_stack.last().unwrap().catalog, base, part)
            }
            None => (&self.position.last().unwrap().catalog, "", input),
        };

        let entries = self.catalog.read_dir(parent)?;

        let mut out = Vec::new();
        for entry in entries {
            let mut name = base.to_string();
            if entry.name.starts_with(part.as_bytes()) {
                name.push_str(std::str::from_utf8(&entry.name)?);
                if entry.is_directory() {
                    name.push('/');
                }
                out.push(name);
            }
        }

        Ok(out)
    }

    // Break async recursion here: lookup -> walk_catalog -> step -> lookup
    fn lookup<'future, 's, 'c, 'a, 'p, 'y>(
        stack: &'s [PathStackEntry],
        catalog: &'c mut CatalogReader,
        accessor: &'a Accessor,
        path: Option<&'p Path>,
        follow_symlinks: &'y mut Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<PathStackEntry>, Error>> + Send + 'future>>
    where
        's: 'future,
        'c: 'future,
        'a: 'future,
        'p: 'future,
        'y: 'future,
    {
        Box::pin(async move {
            Ok(match path {
                None => stack.to_vec(),
                Some(path) => {
                    let mut stack = if path.is_absolute() {
                        stack[..1].to_vec()
                    } else {
                        stack.to_vec()
                    };
                    Self::walk_catalog(&mut stack, catalog, accessor, path, follow_symlinks)
                        .await?;
                    stack
                }
            })
        })
    }

    async fn ls(&mut self, path: Option<&Path>) -> Result<(), Error> {
        let stack = Self::lookup(
            &self.position,
            &mut self.catalog,
            &self.accessor,
            path,
            &mut Some(0),
        )
        .await?;

        let last = stack.last().unwrap();
        if last.catalog.is_directory() {
            let items = self.catalog.read_dir(&stack.last().unwrap().catalog)?;
            let mut out = std::io::stdout();
            // FIXME: columnize
            for item in items {
                out.write_all(&item.name)?;
                out.write_all(b"\n")?;
            }
        } else {
            let mut out = std::io::stdout();
            out.write_all(&last.catalog.name)?;
            out.write_all(b"\n")?;
        }
        Ok(())
    }

    async fn stat(&mut self, path: PathBuf) -> Result<(), Error> {
        let mut stack = Self::lookup(
            &self.position,
            &mut self.catalog,
            &self.accessor,
            Some(&path),
            &mut Some(0),
        )
        .await?;

        let file = Self::walk_pxar_archive(&self.accessor, &mut stack).await?;
        std::io::stdout()
            .write_all(crate::pxar::format_multi_line_entry(file.entry()).as_bytes())?;
        Ok(())
    }

    async fn cd(&mut self, path: Option<&Path>) -> Result<(), Error> {
        match path {
            Some(path) => {
                let new_position = Self::lookup(
                    &self.position,
                    &mut self.catalog,
                    &self.accessor,
                    Some(path),
                    &mut None,
                )
                .await?;
                if !new_position.last().unwrap().catalog.is_directory() {
                    bail!("not a directory");
                }
                self.position = new_position;
            }
            None => self.position.truncate(1),
        }
        self.update_prompt();
        Ok(())
    }

    /// This stack must have been canonicalized already!
    fn format_path_stack(stack: &[PathStackEntry]) -> OsString {
        if stack.len() <= 1 {
            return OsString::from("/");
        }

        let mut out = OsString::new();
        for c in stack.iter().skip(1) {
            out.push("/");
            out.push(OsStr::from_bytes(&c.catalog.name));
        }

        out
    }

    async fn select(&mut self, path: PathBuf) -> Result<(), Error> {
        let stack = Self::lookup(
            &self.position,
            &mut self.catalog,
            &self.accessor,
            Some(&path),
            &mut Some(0),
        )
        .await?;

        let path = Self::format_path_stack(&stack);
        let entry = MatchEntry::include(MatchPattern::Literal(path.as_bytes().to_vec()));
        if self.selected.insert(path.clone(), entry).is_some() {
            println!("path already selected: {:?}", path);
        } else {
            println!("added path: {:?}", path);
        }

        Ok(())
    }

    async fn deselect(&mut self, path: PathBuf) -> Result<(), Error> {
        let stack = Self::lookup(
            &self.position,
            &mut self.catalog,
            &self.accessor,
            Some(&path),
            &mut Some(0),
        )
        .await?;

        let path = Self::format_path_stack(&stack);

        if self.selected.remove(&path).is_some() {
            println!("removed path from selection: {:?}", path);
        } else {
            println!("path not selected: {:?}", path);
        }

        Ok(())
    }

    async fn deselect_all(&mut self) -> Result<(), Error> {
        self.selected.clear();
        println!("cleared selection");
        Ok(())
    }

    async fn list_selected(&mut self, patterns: bool) -> Result<(), Error> {
        if patterns {
            self.list_selected_patterns().await
        } else {
            self.list_matching_files().await
        }
    }

    async fn list_selected_patterns(&self) -> Result<(), Error> {
        for entry in self.selected.keys() {
            println!("{:?}", entry);
        }
        Ok(())
    }

    fn build_match_list(&self) -> Vec<MatchEntry> {
        let mut list = Vec::with_capacity(self.selected.len());
        for entry in self.selected.values() {
            list.push(entry.clone());
        }
        list
    }

    async fn list_matching_files(&mut self) -> Result<(), Error> {
        let matches = self.build_match_list();

        self.catalog.find(
            &self.position[0].catalog,
            &mut Vec::new(),
            &matches,
            &mut |path: &[u8]| -> Result<(), Error> {
                let mut out = std::io::stdout();
                out.write_all(path)?;
                out.write_all(b"\n")?;
                Ok(())
            },
        )?;

        Ok(())
    }

    async fn find(&mut self, pattern: String, select: bool) -> Result<(), Error> {
        let pattern_os = OsString::from(pattern.clone());
        let pattern_entry =
            MatchEntry::parse_pattern(pattern, PatternFlag::PATH_NAME, MatchType::Include)?;

        let mut found_some = false;
        self.catalog.find(
            &self.position[0].catalog,
            &mut Vec::new(),
            &[&pattern_entry],
            &mut |path: &[u8]| -> Result<(), Error> {
                found_some = true;
                let mut out = std::io::stdout();
                out.write_all(path)?;
                out.write_all(b"\n")?;
                Ok(())
            },
        )?;

        if found_some && select {
            self.selected.insert(pattern_os, pattern_entry);
        }

        Ok(())
    }

    async fn restore_selected(&mut self, destination: PathBuf) -> Result<(), Error> {
        if self.selected.is_empty() {
            bail!("no entries selected");
        }

        let match_list = self.build_match_list();

        self.restore_with_match_list(destination, &match_list).await
    }

    async fn restore(
        &mut self,
        destination: PathBuf,
        pattern: Option<String>,
    ) -> Result<(), Error> {
        let tmp;
        let match_list: &[MatchEntry] = match pattern {
            None => &[],
            Some(pattern) => {
                tmp = [MatchEntry::parse_pattern(
                    pattern,
                    PatternFlag::PATH_NAME,
                    MatchType::Include,
                )?];
                &tmp
            }
        };

        self.restore_with_match_list(destination, match_list).await
    }

    async fn restore_with_match_list(
        &mut self,
        destination: PathBuf,
        match_list: &[MatchEntry],
    ) -> Result<(), Error> {
        create_path(
            &destination,
            None,
            Some(CreateOptions::new().perm(Mode::from_bits_truncate(0o700))),
        )
        .map_err(|err| format_err!("error creating directory {:?}: {}", destination, err))?;

        let rootdir = Dir::open(
            &destination,
            OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .map_err(|err| {
            format_err!("unable to open target directory {:?}: {}", destination, err,)
        })?;

        let mut dir_stack = self.new_path_stack();
        Self::walk_pxar_archive(&self.accessor, &mut dir_stack).await?;
        let root_meta = dir_stack
            .last()
            .unwrap()
            .pxar
            .as_ref()
            .unwrap()
            .entry()
            .metadata()
            .clone();

        let extractor = crate::pxar::extract::Extractor::new(
            rootdir,
            root_meta,
            true,
            crate::pxar::extract::OverwriteFlags::empty(),
            Flags::DEFAULT,
        );

        let mut extractor = ExtractorState::new(
            &mut self.catalog,
            dir_stack,
            extractor,
            match_list,
            &self.accessor,
        )?;

        extractor.extract().await
    }
}

struct ExtractorState<'a> {
    path: Vec<u8>,
    path_len: usize,
    path_len_stack: Vec<usize>,

    dir_stack: Vec<PathStackEntry>,

    matches: bool,
    matches_stack: Vec<bool>,

    read_dir: <Vec<catalog::DirEntry> as IntoIterator>::IntoIter,
    read_dir_stack: Vec<<Vec<catalog::DirEntry> as IntoIterator>::IntoIter>,

    extractor: crate::pxar::extract::Extractor,

    catalog: &'a mut CatalogReader,
    match_list: &'a [MatchEntry],
    accessor: &'a Accessor,
}

impl<'a> ExtractorState<'a> {
    pub fn new(
        catalog: &'a mut CatalogReader,
        dir_stack: Vec<PathStackEntry>,
        extractor: crate::pxar::extract::Extractor,
        match_list: &'a [MatchEntry],
        accessor: &'a Accessor,
    ) -> Result<Self, Error> {
        let read_dir = catalog
            .read_dir(&dir_stack.last().unwrap().catalog)?
            .into_iter();
        Ok(Self {
            path: Vec::new(),
            path_len: 0,
            path_len_stack: Vec::new(),

            dir_stack,

            matches: match_list.is_empty(),
            matches_stack: Vec::new(),

            read_dir,
            read_dir_stack: Vec::new(),

            extractor,

            catalog,
            match_list,
            accessor,
        })
    }

    pub async fn extract(&mut self) -> Result<(), Error> {
        loop {
            let entry = match self.read_dir.next() {
                Some(entry) => entry,
                None => match self.handle_end_of_directory()? {
                    ControlFlow::Break(()) => break, // done with root directory
                    ControlFlow::Continue(()) => continue,
                },
            };

            self.path.truncate(self.path_len);
            if !entry.name.starts_with(b"/") {
                self.path.reserve(entry.name.len() + 1);
                self.path.push(b'/');
            }
            self.path.extend(&entry.name);

            self.extractor
                .set_path(OsString::from_vec(self.path.clone()));
            self.handle_entry(entry).await?;
        }

        Ok(())
    }

    fn handle_end_of_directory(&mut self) -> Result<ControlFlow<()>, Error> {
        // go up a directory:
        self.read_dir = match self.read_dir_stack.pop() {
            Some(r) => r,
            None => return Ok(ControlFlow::Break(())), // out of root directory
        };

        self.matches = self
            .matches_stack
            .pop()
            .ok_or_else(|| format_err!("internal iterator error (matches_stack)"))?;

        self.dir_stack
            .pop()
            .ok_or_else(|| format_err!("internal iterator error (dir_stack)"))?;

        self.path_len = self
            .path_len_stack
            .pop()
            .ok_or_else(|| format_err!("internal iterator error (path_len_stack)"))?;

        self.extractor.leave_directory()?;

        Ok(ControlFlow::Continue(()))
    }

    async fn handle_new_directory(
        &mut self,
        entry: catalog::DirEntry,
        match_result: Option<MatchType>,
    ) -> Result<(), Error> {
        // enter a new directory:
        self.read_dir_stack.push(mem::replace(
            &mut self.read_dir,
            self.catalog.read_dir(&entry)?.into_iter(),
        ));
        self.matches_stack.push(self.matches);
        self.dir_stack.push(PathStackEntry::new(entry));
        self.path_len_stack.push(self.path_len);
        self.path_len = self.path.len();

        Shell::walk_pxar_archive(self.accessor, &mut self.dir_stack).await?;
        let dir_pxar = self.dir_stack.last().unwrap().pxar.as_ref().unwrap();
        let dir_meta = dir_pxar.entry().metadata().clone();
        let create = self.matches && match_result != Some(MatchType::Exclude);
        self.extractor
            .enter_directory(dir_pxar.file_name().to_os_string(), dir_meta, create)?;

        Ok(())
    }

    pub async fn handle_entry(&mut self, entry: catalog::DirEntry) -> Result<(), Error> {
        let match_result = self.match_list.matches(&self.path, entry.get_file_mode());
        let did_match = match match_result {
            Ok(Some(MatchType::Include)) => true,
            Ok(Some(MatchType::Exclude)) => false,
            _ => self.matches,
        };

        match (did_match, &entry.attr) {
            (_, DirEntryAttribute::Directory { .. }) => {
                self.handle_new_directory(entry, match_result?).await?;
            }
            (true, DirEntryAttribute::File { .. }) => {
                self.dir_stack.push(PathStackEntry::new(entry));
                let file = Shell::walk_pxar_archive(self.accessor, &mut self.dir_stack).await?;
                self.extract_file(file).await?;
                self.dir_stack.pop();
            }
            (true, DirEntryAttribute::Symlink)
            | (true, DirEntryAttribute::BlockDevice)
            | (true, DirEntryAttribute::CharDevice)
            | (true, DirEntryAttribute::Fifo)
            | (true, DirEntryAttribute::Socket)
            | (true, DirEntryAttribute::Hardlink) => {
                let attr = entry.attr.clone();
                self.dir_stack.push(PathStackEntry::new(entry));
                let file = Shell::walk_pxar_archive(self.accessor, &mut self.dir_stack).await?;
                self.extract_special(file, attr).await?;
                self.dir_stack.pop();
            }
            (false, _) => (), // skip
        }

        Ok(())
    }

    fn path(&self) -> &OsStr {
        OsStr::from_bytes(&self.path)
    }

    async fn extract_file(&mut self, entry: FileEntry) -> Result<(), Error> {
        match entry.kind() {
            pxar::EntryKind::File { size, .. } => {
                let file_name = CString::new(entry.file_name().as_bytes())?;
                let mut contents = entry.contents().await?;
                self.extractor
                    .async_extract_file(&file_name, entry.metadata(), *size, &mut contents, false)
                    .await
            }
            _ => {
                bail!(
                    "catalog file {:?} not a regular file in the archive",
                    self.path()
                );
            }
        }
    }

    async fn extract_special(
        &mut self,
        entry: FileEntry,
        catalog_attr: DirEntryAttribute,
    ) -> Result<(), Error> {
        let file_name = CString::new(entry.file_name().as_bytes())?;
        match (catalog_attr, entry.kind()) {
            (DirEntryAttribute::Symlink, pxar::EntryKind::Symlink(symlink)) => {
                block_in_place(|| {
                    self.extractor.extract_symlink(
                        &file_name,
                        entry.metadata(),
                        symlink.as_os_str(),
                    )
                })
            }
            (DirEntryAttribute::Symlink, _) => {
                bail!(
                    "catalog symlink {:?} not a symlink in the archive",
                    self.path()
                );
            }

            (DirEntryAttribute::Hardlink, pxar::EntryKind::Hardlink(hardlink)) => {
                block_in_place(|| {
                    self.extractor
                        .extract_hardlink(&file_name, hardlink.as_os_str())
                })
            }
            (DirEntryAttribute::Hardlink, _) => {
                bail!(
                    "catalog hardlink {:?} not a hardlink in the archive",
                    self.path()
                );
            }

            (ref attr, pxar::EntryKind::Device(device)) => {
                self.extract_device(attr.clone(), &file_name, device, entry.metadata())
            }

            (DirEntryAttribute::Fifo, pxar::EntryKind::Fifo) => block_in_place(|| {
                self.extractor
                    .extract_special(&file_name, entry.metadata(), 0)
            }),
            (DirEntryAttribute::Fifo, _) => {
                bail!("catalog fifo {:?} not a fifo in the archive", self.path());
            }

            (DirEntryAttribute::Socket, pxar::EntryKind::Socket) => block_in_place(|| {
                self.extractor
                    .extract_special(&file_name, entry.metadata(), 0)
            }),
            (DirEntryAttribute::Socket, _) => {
                bail!(
                    "catalog socket {:?} not a socket in the archive",
                    self.path()
                );
            }

            attr => bail!("unhandled file type {:?} for {:?}", attr, self.path()),
        }
    }

    fn extract_device(
        &mut self,
        attr: DirEntryAttribute,
        file_name: &CStr,
        device: &pxar::format::Device,
        metadata: &Metadata,
    ) -> Result<(), Error> {
        match attr {
            DirEntryAttribute::BlockDevice => {
                if !metadata.stat.is_blockdev() {
                    bail!(
                        "catalog block device {:?} is not a block device in the archive",
                        self.path(),
                    );
                }
            }
            DirEntryAttribute::CharDevice => {
                if !metadata.stat.is_chardev() {
                    bail!(
                        "catalog character device {:?} is not a character device in the archive",
                        self.path(),
                    );
                }
            }
            _ => {
                bail!(
                    "unexpected file type for {:?} in the catalog, \
                     which is a device special file in the archive",
                    self.path(),
                );
            }
        }
        block_in_place(|| {
            self.extractor
                .extract_special(file_name, metadata, device.to_dev_t())
        })
    }
}
