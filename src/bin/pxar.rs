extern crate proxmox_backup;

use failure::*;

use proxmox::{sortable, identity};
use proxmox::api::{ApiHandler, ApiMethod, RpcEnvironment};
use proxmox::api::schema::*;
use proxmox::api::cli::*;

use proxmox_backup::tools;

use serde_json::{Value};

use std::io::Write;
use std::path::{Path, PathBuf};
use std::fs::OpenOptions;
use std::ffi::OsStr;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::collections::HashSet;

use proxmox_backup::pxar;

fn dump_archive_from_reader<R: std::io::Read>(
    reader: &mut R,
    feature_flags: u64,
    verbose: bool,
) -> Result<(), Error> {
    let mut decoder = pxar::SequentialDecoder::new(reader, feature_flags);

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let mut path = PathBuf::new();
    decoder.dump_entry(&mut path, verbose, &mut out)?;

    Ok(())
}

fn dump_archive(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let archive = tools::required_string_param(&param, "archive")?;
    let verbose = param["verbose"].as_bool().unwrap_or(false);

    let feature_flags = pxar::flags::DEFAULT;

    if archive == "-" {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        dump_archive_from_reader(&mut reader, feature_flags, verbose)?;
    } else {
        if verbose { println!("PXAR dump: {}", archive); }
        let file = std::fs::File::open(archive)?;
        let mut reader = std::io::BufReader::new(file);
        dump_archive_from_reader(&mut reader, feature_flags, verbose)?;
    }

    Ok(Value::Null)
}

fn extract_archive_from_reader<R: std::io::Read>(
    reader: &mut R,
    target: &str,
    feature_flags: u64,
    allow_existing_dirs: bool,
    verbose: bool,
    pattern: Option<Vec<pxar::MatchPattern>>
) -> Result<(), Error> {
    let mut decoder = pxar::SequentialDecoder::new(reader, feature_flags);
    decoder.set_callback(move |path| {
        if verbose {
            println!("{:?}", path);
        }
        Ok(())
    });
    decoder.set_allow_existing_dirs(allow_existing_dirs);

    let pattern = pattern.unwrap_or_else(Vec::new);
    decoder.restore(Path::new(target), &pattern)?;

    Ok(())
}

fn extract_archive(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let archive = tools::required_string_param(&param, "archive")?;
    let target = param["target"].as_str().unwrap_or(".");
    let verbose = param["verbose"].as_bool().unwrap_or(false);
    let no_xattrs = param["no-xattrs"].as_bool().unwrap_or(false);
    let no_fcaps = param["no-fcaps"].as_bool().unwrap_or(false);
    let no_acls = param["no-acls"].as_bool().unwrap_or(false);
    let no_device_nodes = param["no-device-nodes"].as_bool().unwrap_or(false);
    let no_fifos = param["no-fifos"].as_bool().unwrap_or(false);
    let no_sockets = param["no-sockets"].as_bool().unwrap_or(false);
    let allow_existing_dirs = param["allow-existing-dirs"].as_bool().unwrap_or(false);
    let files_from = param["files-from"].as_str();
    let empty = Vec::new();
    let arg_pattern = param["pattern"].as_array().unwrap_or(&empty);

    let mut feature_flags = pxar::flags::DEFAULT;
    if no_xattrs {
        feature_flags ^= pxar::flags::WITH_XATTRS;
    }
    if no_fcaps {
        feature_flags ^= pxar::flags::WITH_FCAPS;
    }
    if no_acls {
        feature_flags ^= pxar::flags::WITH_ACL;
    }
    if no_device_nodes {
        feature_flags ^= pxar::flags::WITH_DEVICE_NODES;
    }
    if no_fifos {
        feature_flags ^= pxar::flags::WITH_FIFOS;
    }
    if no_sockets {
        feature_flags ^= pxar::flags::WITH_SOCKETS;
    }

    let mut pattern_list = Vec::new();
    if let Some(filename) = files_from {
        let dir = nix::dir::Dir::open("./", nix::fcntl::OFlag::O_RDONLY, nix::sys::stat::Mode::empty())?;
        if let Some((mut pattern, _, _)) = pxar::MatchPattern::from_file(dir.as_raw_fd(), filename)? {
            pattern_list.append(&mut pattern);
        }
    }

    for s in arg_pattern {
        let l = s.as_str().ok_or_else(|| format_err!("Invalid pattern string slice"))?;
        let p = pxar::MatchPattern::from_line(l.as_bytes())?
            .ok_or_else(|| format_err!("Invalid match pattern in arguments"))?;
        pattern_list.push(p);
    }

    let pattern = if pattern_list.is_empty() {
        None
    } else {
        Some(pattern_list)
    };

    if archive == "-" {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        extract_archive_from_reader(&mut reader, target, feature_flags, allow_existing_dirs, verbose, pattern)?;
    } else {
        if verbose { println!("PXAR extract: {}", archive); }
        let file = std::fs::File::open(archive)?;
        let mut reader = std::io::BufReader::new(file);
        extract_archive_from_reader(&mut reader, target, feature_flags, allow_existing_dirs, verbose, pattern)?;
    }

    Ok(Value::Null)
}

fn create_archive(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let archive = tools::required_string_param(&param, "archive")?;
    let source = tools::required_string_param(&param, "source")?;
    let verbose = param["verbose"].as_bool().unwrap_or(false);
    let all_file_systems = param["all-file-systems"].as_bool().unwrap_or(false);
    let no_xattrs = param["no-xattrs"].as_bool().unwrap_or(false);
    let no_fcaps = param["no-fcaps"].as_bool().unwrap_or(false);
    let no_acls = param["no-acls"].as_bool().unwrap_or(false);
    let no_device_nodes = param["no-device-nodes"].as_bool().unwrap_or(false);
    let no_fifos = param["no-fifos"].as_bool().unwrap_or(false);
    let no_sockets = param["no-sockets"].as_bool().unwrap_or(false);
    let empty = Vec::new();
    let exclude_pattern = param["exclude"].as_array().unwrap_or(&empty);
    let entries_max = param["entries-max"].as_u64().unwrap_or(pxar::ENCODER_MAX_ENTRIES as u64);

    let devices = if all_file_systems { None } else { Some(HashSet::new()) };

    let source = PathBuf::from(source);

    let mut dir = nix::dir::Dir::open(
        &source, nix::fcntl::OFlag::O_NOFOLLOW, nix::sys::stat::Mode::empty())?;

    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o640)
        .open(archive)?;

    let mut writer = std::io::BufWriter::with_capacity(1024*1024, file);
    let mut feature_flags = pxar::flags::DEFAULT;
    if no_xattrs {
        feature_flags ^= pxar::flags::WITH_XATTRS;
    }
    if no_fcaps {
        feature_flags ^= pxar::flags::WITH_FCAPS;
    }
    if no_acls {
        feature_flags ^= pxar::flags::WITH_ACL;
    }
    if no_device_nodes {
        feature_flags ^= pxar::flags::WITH_DEVICE_NODES;
    }
    if no_fifos {
        feature_flags ^= pxar::flags::WITH_FIFOS;
    }
    if no_sockets {
        feature_flags ^= pxar::flags::WITH_SOCKETS;
    }

    let mut pattern_list = Vec::new();
    for s in exclude_pattern {
        let l = s.as_str().ok_or_else(|| format_err!("Invalid pattern string slice"))?;
        let p = pxar::MatchPattern::from_line(l.as_bytes())?
            .ok_or_else(|| format_err!("Invalid match pattern in arguments"))?;
        pattern_list.push(p);
    }

    let catalog = None::<&mut pxar::catalog::DummyCatalogWriter>;
    pxar::Encoder::encode(
        source,
        &mut dir,
        &mut writer,
        catalog,
        devices,
        verbose,
        false,
        feature_flags,
        pattern_list,
        entries_max as usize,
    )?;

    writer.flush()?;

    Ok(Value::Null)
}

/// Mount the archive to the provided mountpoint via FUSE.
fn mount_archive(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let archive = tools::required_string_param(&param, "archive")?;
    let mountpoint = tools::required_string_param(&param, "mountpoint")?;
    let verbose = param["verbose"].as_bool().unwrap_or(false);
    let no_mt = param["no-mt"].as_bool().unwrap_or(false);

    let archive = Path::new(archive);
    let mountpoint = Path::new(mountpoint);
    let options = OsStr::new("ro,default_permissions");
    let mut session = pxar::fuse::Session::from_path(&archive, &options, verbose)
        .map_err(|err| format_err!("pxar mount failed: {}", err))?;
    // Mount the session and deamonize if verbose is not set
    session.mount(&mountpoint, !verbose)?;
    session.run_loop(!no_mt)?;

    Ok(Value::Null)
}

#[sortable]
const API_METHOD_CREATE_ARCHIVE: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&create_archive),
    &ObjectSchema::new(
        "Create new .pxar archive.",
        &sorted!([
            (
                "archive",
                false,
                &StringSchema::new("Archive name").schema()
            ),
            (
                "source",
                false,
                &StringSchema::new("Source directory.").schema()
            ),
            (
                "verbose",
                true,
                &BooleanSchema::new("Verbose output.")
                    .default(false)
                    .schema()
            ),
            (
                "no-xattrs",
                true,
                &BooleanSchema::new("Ignore extended file attributes.")
                    .default(false)
                    .schema()
            ),
            (
                "no-fcaps",
                true,
                &BooleanSchema::new("Ignore file capabilities.")
                    .default(false)
                    .schema()
            ),
            (
                "no-acls",
                true,
                &BooleanSchema::new("Ignore access control list entries.")
                    .default(false)
                    .schema()
            ),
            (
                "all-file-systems",
                true,
                &BooleanSchema::new("Include mounted sudirs.")
                    .default(false)
                    .schema()
            ),
            (
                "no-device-nodes",
                true,
                &BooleanSchema::new("Ignore device nodes.")
                    .default(false)
                    .schema()
            ),
            (
                "no-fifos",
                true,
                &BooleanSchema::new("Ignore fifos.")
                    .default(false)
                    .schema()
            ),
            (
                "no-sockets",
                true,
                &BooleanSchema::new("Ignore sockets.")
                    .default(false)
                    .schema()
            ),
            (
                "exclude",
                true,
                &ArraySchema::new(
                    "List of paths or pattern matching files to exclude.",
                    &StringSchema::new("Path or pattern matching files to restore.").schema()
                ).schema()
            ),
            (
                "entries-max",
                true,
                &IntegerSchema::new("Max number of entries loaded at once into memory")
                    .default(pxar::ENCODER_MAX_ENTRIES as isize)
                    .minimum(0)
                    .maximum(std::isize::MAX)
                    .schema()
            ),
        ]),
    )
);

#[sortable]
const API_METHOD_EXTRACT_ARCHIVE: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&extract_archive),
    &ObjectSchema::new(
        "Extract an archive.",
        &sorted!([
            (
                "archive",
                false,
                &StringSchema::new("Archive name.").schema()
            ),
            (
                "pattern",
                true,
                &ArraySchema::new(
                    "List of paths or pattern matching files to restore",
                    &StringSchema::new("Path or pattern matching files to restore.").schema()
                ).schema()
            ),
            (
                "target",
                true,
                &StringSchema::new("Target directory.").schema()
            ),
            (
                "verbose",
                true,
                &BooleanSchema::new("Verbose output.")
                    .default(false)
                    .schema()
            ),
            (
                "no-xattrs",
                true,
                &BooleanSchema::new("Ignore extended file attributes.")
                    .default(false)
                    .schema()
            ),
            (
                "no-fcaps",
                true,
                &BooleanSchema::new("Ignore file capabilities.")
                    .default(false)
                    .schema()
            ),
            (
                "no-acls",
                true,
                &BooleanSchema::new("Ignore access control list entries.")
                    .default(false)
                    .schema()
            ),
            (
                "allow-existing-dirs",
                true,
                &BooleanSchema::new("Allows directories to already exist on restore.")
                    .default(false)
                    .schema()
            ),
            (
                "files-from",
                true,
                &StringSchema::new("Match pattern for files to restore.").schema()
            ),
            (
                "no-device-nodes",
                true,
                &BooleanSchema::new("Ignore device nodes.")
                    .default(false)
                    .schema()
            ),
            (
                "no-fifos",
                true,
                &BooleanSchema::new("Ignore fifos.")
                    .default(false)
                    .schema()
            ),
            (
                "no-sockets",
                true,
                &BooleanSchema::new("Ignore sockets.")
                    .default(false)
                    .schema()
            ),
        ]),
    )
);

#[sortable]
const API_METHOD_MOUNT_ARCHIVE: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&mount_archive),
    &ObjectSchema::new(
        "Mount the archive as filesystem via FUSE.",
        &sorted!([
            (
                "archive",
                false,
                &StringSchema::new("Archive name.").schema()
            ),
            (
                "mountpoint",
                false,
                &StringSchema::new("Mountpoint for the filesystem root.").schema()
            ),
            (
                "verbose",
                true,
                &BooleanSchema::new("Verbose output, keeps process running in foreground (for debugging).")
                    .default(false)
                    .schema()
            ),
            (
                "no-mt",
                true,
                &BooleanSchema::new("Run in single threaded mode (for debugging).")
                    .default(false)
                    .schema()
            ),
        ]),
    )
);

#[sortable]
const API_METHOD_DUMP_ARCHIVE: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&dump_archive),
    &ObjectSchema::new(
        "List the contents of an archive.",
        &sorted!([
            ( "archive", false, &StringSchema::new("Archive name.").schema()),
            ( "verbose", true, &BooleanSchema::new("Verbose output.")
               .default(false)
               .schema()
            ),
        ])
    )
);

fn main() {

    let cmd_def = CliCommandMap::new()
        .insert("create", CliCommand::new(&API_METHOD_CREATE_ARCHIVE)
            .arg_param(&["archive", "source"])
            .completion_cb("archive", tools::complete_file_name)
            .completion_cb("source", tools::complete_file_name)
        )
        .insert("extract", CliCommand::new(&API_METHOD_EXTRACT_ARCHIVE)
            .arg_param(&["archive", "target"])
            .completion_cb("archive", tools::complete_file_name)
            .completion_cb("target", tools::complete_file_name)
            .completion_cb("files-from", tools::complete_file_name)
         )
        .insert("mount", CliCommand::new(&API_METHOD_MOUNT_ARCHIVE)
            .arg_param(&["archive", "mountpoint"])
            .completion_cb("archive", tools::complete_file_name)
            .completion_cb("mountpoint", tools::complete_file_name)
        )
        .insert("list", CliCommand::new(&API_METHOD_DUMP_ARCHIVE)
            .arg_param(&["archive"])
            .completion_cb("archive", tools::complete_file_name)
        );

    run_cli_command(cmd_def, None);
}
