use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, format_err, Error};
use futures::future::FutureExt;
use futures::select;
use tokio::signal::unix::{signal, SignalKind};

use pathpatterns::{MatchEntry, MatchType, PatternFlag};

use proxmox::api::cli::*;
use proxmox::api::api;

use proxmox_backup::tools;
use proxmox_backup::pxar::{fuse, format_single_line_entry, ENCODER_MAX_ENTRIES, Flags, PxarExtractOptions};

fn extract_archive_from_reader<R: std::io::Read>(
    reader: &mut R,
    target: &str,
    feature_flags: Flags,
    verbose: bool,
    options: PxarExtractOptions,
) -> Result<(), Error> {

    proxmox_backup::pxar::extract_archive(
        pxar::decoder::Decoder::from_std(reader)?,
        Path::new(target),
        feature_flags,
        |path| {
            if verbose {
                println!("{:?}", path);
            }
        },
        options,
    )
}

#[api(
    input: {
        properties: {
            archive: {
                description: "Archive name.",
            },
            pattern: {
                description: "List of paths or pattern matching files to restore",
                type: Array,
                items: {
                    type: String,
                    description: "Path or pattern matching files to restore.",
                },
                optional: true,
            },
            target: {
                description: "Target directory",
                optional: true,
            },
            verbose: {
                description: "Verbose output.",
                optional: true,
                default: false,
            },
            "no-xattrs": {
                description: "Ignore extended file attributes.",
                optional: true,
                default: false,
            },
            "no-fcaps": {
                description: "Ignore file capabilities.",
                optional: true,
                default: false,
            },
            "no-acls": {
                description: "Ignore access control list entries.",
                optional: true,
                default: false,
            },
            "allow-existing-dirs": {
                description: "Allows directories to already exist on restore.",
                optional: true,
                default: false,
            },
            "files-from": {
                description: "File containing match pattern for files to restore.",
                optional: true,
            },
            "no-device-nodes": {
                description: "Ignore device nodes.",
                optional: true,
                default: false,
            },
            "no-fifos": {
                description: "Ignore fifos.",
                optional: true,
                default: false,
            },
            "no-sockets": {
                description: "Ignore sockets.",
                optional: true,
                default: false,
            },
            strict: {
                description: "Stop on errors. Otherwise most errors will simply warn.",
                optional: true,
                default: false,
            },
        },
    },
)]
/// Extract an archive.
#[allow(clippy::too_many_arguments)]
fn extract_archive(
    archive: String,
    pattern: Option<Vec<String>>,
    target: Option<String>,
    verbose: bool,
    no_xattrs: bool,
    no_fcaps: bool,
    no_acls: bool,
    allow_existing_dirs: bool,
    files_from: Option<String>,
    no_device_nodes: bool,
    no_fifos: bool,
    no_sockets: bool,
    strict: bool,
) -> Result<(), Error> {
    let mut feature_flags = Flags::DEFAULT;
    if no_xattrs {
        feature_flags.remove(Flags::WITH_XATTRS);
    }
    if no_fcaps {
        feature_flags.remove(Flags::WITH_FCAPS);
    }
    if no_acls {
        feature_flags.remove(Flags::WITH_ACL);
    }
    if no_device_nodes {
        feature_flags.remove(Flags::WITH_DEVICE_NODES);
    }
    if no_fifos {
        feature_flags.remove(Flags::WITH_FIFOS);
    }
    if no_sockets {
        feature_flags.remove(Flags::WITH_SOCKETS);
    }

    let pattern = pattern.unwrap_or_else(Vec::new);
    let target = target.as_ref().map_or_else(|| ".", String::as_str);

    let mut match_list = Vec::new();
    if let Some(filename) = &files_from {
        for line in proxmox_backup::tools::file_get_non_comment_lines(filename)? {
            let line = line
                .map_err(|err| format_err!("error reading {}: {}", filename, err))?;
            match_list.push(
                MatchEntry::parse_pattern(line, PatternFlag::PATH_NAME, MatchType::Include)
                    .map_err(|err| format_err!("bad pattern in file '{}': {}", filename, err))?,
            );
        }
    }

    for entry in pattern {
        match_list.push(
            MatchEntry::parse_pattern(entry, PatternFlag::PATH_NAME, MatchType::Include)
                .map_err(|err| format_err!("error in pattern: {}", err))?,
        );
    }

    let extract_match_default = match_list.is_empty();

    let was_ok = Arc::new(AtomicBool::new(true));
    let on_error = if strict {
        // by default errors are propagated up
        None
    } else {
        let was_ok = Arc::clone(&was_ok);
        // otherwise we want to log them but not act on them
        Some(Box::new(move |err| {
            was_ok.store(false, Ordering::Release);
            eprintln!("error: {}", err);
            Ok(())
        }) as Box<dyn FnMut(Error) -> Result<(), Error> + Send>)
    };

    let options = PxarExtractOptions {
        match_list: &match_list,
        allow_existing_dirs,
        extract_match_default,
        on_error,
    };

    if archive == "-" {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        extract_archive_from_reader(
            &mut reader,
            &target,
            feature_flags,
            verbose,
            options,
        )?;
    } else {
        if verbose {
            println!("PXAR extract: {}", archive);
        }
        let file = std::fs::File::open(archive)?;
        let mut reader = std::io::BufReader::new(file);
        extract_archive_from_reader(
            &mut reader,
            &target,
            feature_flags,
            verbose,
            options,
        )?;
    }

    if !was_ok.load(Ordering::Acquire) {
        bail!("there were errors");
    }

    Ok(())
}

#[api(
    input: {
        properties: {
            archive: {
                description: "Archive name.",
            },
            source: {
                description: "Source directory.",
            },
            verbose: {
                description: "Verbose output.",
                optional: true,
                default: false,
            },
            "no-xattrs": {
                description: "Ignore extended file attributes.",
                optional: true,
                default: false,
            },
            "no-fcaps": {
                description: "Ignore file capabilities.",
                optional: true,
                default: false,
            },
            "no-acls": {
                description: "Ignore access control list entries.",
                optional: true,
                default: false,
            },
            "all-file-systems": {
                description: "Include mounted sudirs.",
                optional: true,
                default: false,
            },
            "no-device-nodes": {
                description: "Ignore device nodes.",
                optional: true,
                default: false,
            },
            "no-fifos": {
                description: "Ignore fifos.",
                optional: true,
                default: false,
            },
            "no-sockets": {
                description: "Ignore sockets.",
                optional: true,
                default: false,
            },
            exclude: {
                description: "List of paths or pattern matching files to exclude.",
                optional: true,
                type: Array,
                items: {
                    description: "Path or pattern matching files to restore",
                    type: String,
                },
            },
            "entries-max": {
                description: "Max number of entries loaded at once into memory",
                optional: true,
                default: ENCODER_MAX_ENTRIES as isize,
                minimum: 0,
                maximum: std::isize::MAX,
            },
        },
    },
)]
/// Create a new .pxar archive.
#[allow(clippy::too_many_arguments)]
async fn create_archive(
    archive: String,
    source: String,
    verbose: bool,
    no_xattrs: bool,
    no_fcaps: bool,
    no_acls: bool,
    all_file_systems: bool,
    no_device_nodes: bool,
    no_fifos: bool,
    no_sockets: bool,
    exclude: Option<Vec<String>>,
    entries_max: isize,
) -> Result<(), Error> {
    let patterns = {
        let input = exclude.unwrap_or_else(Vec::new);
        let mut patterns = Vec::with_capacity(input.len());
        for entry in input {
            patterns.push(
                MatchEntry::parse_pattern(entry, PatternFlag::PATH_NAME, MatchType::Exclude)
                    .map_err(|err| format_err!("error in exclude pattern: {}", err))?,
            );
        }
        patterns
    };

    let device_set = if all_file_systems {
        None
    } else {
        Some(HashSet::new())
    };

    let options = proxmox_backup::pxar::PxarCreateOptions {
        entries_max: entries_max as usize,
        device_set,
        patterns,
        verbose,
        skip_lost_and_found: false,
    };


    let source = PathBuf::from(source);

    let dir = nix::dir::Dir::open(
        &source,
        nix::fcntl::OFlag::O_NOFOLLOW,
        nix::sys::stat::Mode::empty(),
    )?;

    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o640)
        .open(archive)?;

    let writer = std::io::BufWriter::with_capacity(1024 * 1024, file);
    let mut feature_flags = Flags::DEFAULT;
    if no_xattrs {
        feature_flags.remove(Flags::WITH_XATTRS);
    }
    if no_fcaps {
        feature_flags.remove(Flags::WITH_FCAPS);
    }
    if no_acls {
        feature_flags.remove(Flags::WITH_ACL);
    }
    if no_device_nodes {
        feature_flags.remove(Flags::WITH_DEVICE_NODES);
    }
    if no_fifos {
        feature_flags.remove(Flags::WITH_FIFOS);
    }
    if no_sockets {
        feature_flags.remove(Flags::WITH_SOCKETS);
    }

    let writer = pxar::encoder::sync::StandardWriter::new(writer);
    proxmox_backup::pxar::create_archive(
        dir,
        writer,
        feature_flags,
        move |path| {
            if verbose {
                println!("{:?}", path);
            }
            Ok(())
        },
        None,
        options,
    ).await?;

    Ok(())
}

#[api(
    input: {
        properties: {
            archive: { description: "Archive name." },
            mountpoint: { description: "Mountpoint for the file system." },
            verbose: {
                description: "Verbose output, running in the foreground (for debugging).",
                optional: true,
                default: false,
            },
        },
    },
)]
/// Mount the archive to the provided mountpoint via FUSE.
async fn mount_archive(
    archive: String,
    mountpoint: String,
    verbose: bool,
) -> Result<(), Error> {
    let archive = Path::new(&archive);
    let mountpoint = Path::new(&mountpoint);
    let options = OsStr::new("ro,default_permissions");

    let session = fuse::Session::mount_path(&archive, &options, verbose, mountpoint)
        .await
        .map_err(|err| format_err!("pxar mount failed: {}", err))?;

    let mut interrupt = signal(SignalKind::interrupt())?;

    select! {
        res = session.fuse() => res?,
        _ = interrupt.recv().fuse() => {
            if verbose {
                eprintln!("interrupted");
            }
        }
    }

    Ok(())
}

#[api(
    input: {
        properties: {
            archive: {
                description: "Archive name.",
            },
            verbose: {
                description: "Verbose output.",
                optional: true,
                default: false,
            },
        },
    },
)]
/// List the contents of an archive.
fn dump_archive(archive: String, verbose: bool) -> Result<(), Error> {
    for entry in pxar::decoder::Decoder::open(archive)? {
        let entry = entry?;

        if verbose {
            println!("{}", format_single_line_entry(&entry));
        } else {
            println!("{:?}", entry.path());
        }
    }
    Ok(())
}

fn main() {
    let cmd_def = CliCommandMap::new()
        .insert(
            "create",
            CliCommand::new(&API_METHOD_CREATE_ARCHIVE)
                .arg_param(&["archive", "source"])
                .completion_cb("archive", tools::complete_file_name)
                .completion_cb("source", tools::complete_file_name),
        )
        .insert(
            "extract",
            CliCommand::new(&API_METHOD_EXTRACT_ARCHIVE)
                .arg_param(&["archive", "target"])
                .completion_cb("archive", tools::complete_file_name)
                .completion_cb("target", tools::complete_file_name)
                .completion_cb("files-from", tools::complete_file_name),
        )
        .insert(
            "mount",
            CliCommand::new(&API_METHOD_MOUNT_ARCHIVE)
                .arg_param(&["archive", "mountpoint"])
                .completion_cb("archive", tools::complete_file_name)
                .completion_cb("mountpoint", tools::complete_file_name),
        )
        .insert(
            "list",
            CliCommand::new(&API_METHOD_DUMP_ARCHIVE)
                .arg_param(&["archive"])
                .completion_cb("archive", tools::complete_file_name),
        );

    let rpcenv = CliEnvironment::new();
    run_cli_command(cmd_def, rpcenv, Some(|future| {
        proxmox_backup::tools::runtime::main(future)
    }));
}
