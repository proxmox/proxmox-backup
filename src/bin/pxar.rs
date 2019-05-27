extern crate proxmox_backup;

use failure::*;

use proxmox_backup::tools;
use proxmox_backup::cli::*;
use proxmox_backup::api_schema::*;
use proxmox_backup::api_schema::router::*;

use serde_json::{Value};

use std::io::Write;
use std::path::{Path, PathBuf};
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;

use proxmox_backup::pxar;

fn print_filenames(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let archive = tools::required_string_param(&param, "archive")?;
    let file = std::fs::File::open(archive)?;

    let mut reader = std::io::BufReader::new(file);

    let mut feature_flags = pxar::CA_FORMAT_DEFAULT;
    feature_flags ^= pxar::CA_FORMAT_WITH_XATTRS;
    feature_flags ^= pxar::CA_FORMAT_WITH_FCAPS;
    let mut decoder = pxar::SequentialDecoder::new(&mut reader, feature_flags);

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let mut path = PathBuf::from(".");
    decoder.dump_entry(&mut path, false, &mut out)?;

    Ok(Value::Null)
}

fn dump_archive(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let archive = tools::required_string_param(&param, "archive")?;
    let file = std::fs::File::open(archive)?;

    let mut reader = std::io::BufReader::new(file);

    let mut feature_flags = pxar::CA_FORMAT_DEFAULT;
    feature_flags ^= pxar::CA_FORMAT_WITH_XATTRS;
    feature_flags ^= pxar::CA_FORMAT_WITH_FCAPS;
    let mut decoder = pxar::SequentialDecoder::new(&mut reader, feature_flags);

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    println!("PXAR dump: {}", archive);

    let mut path = PathBuf::new();
    decoder.dump_entry(&mut path, true, &mut out)?;

    Ok(Value::Null)
}

fn extract_archive(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let archive = tools::required_string_param(&param, "archive")?;
    let target = tools::required_string_param(&param, "target")?;
    let verbose = param["verbose"].as_bool().unwrap_or(false);
    let no_xattrs = param["no-xattrs"].as_bool().unwrap_or(false);
    let no_fcaps = param["no-fcaps"].as_bool().unwrap_or(false);

    let file = std::fs::File::open(archive)?;

    let mut reader = std::io::BufReader::new(file);
    let mut feature_flags = pxar::CA_FORMAT_DEFAULT;
    if no_xattrs {
        feature_flags ^= pxar::CA_FORMAT_WITH_XATTRS;
    }
    if no_fcaps {
        feature_flags ^= pxar::CA_FORMAT_WITH_FCAPS;
    }

    let mut decoder = pxar::SequentialDecoder::new(&mut reader, feature_flags);

    decoder.restore(Path::new(target), & |path| {
        if verbose {
            println!("{:?}", path);
        }
        Ok(())
    })?;

    Ok(Value::Null)
}

fn create_archive(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let archive = tools::required_string_param(&param, "archive")?;
    let source = tools::required_string_param(&param, "source")?;
    let verbose = param["verbose"].as_bool().unwrap_or(false);
    let all_file_systems = param["all-file-systems"].as_bool().unwrap_or(false);
    let no_xattrs = param["no-xattrs"].as_bool().unwrap_or(false);
    let no_fcaps = param["no-fcaps"].as_bool().unwrap_or(false);

    let source = PathBuf::from(source);

    let mut dir = nix::dir::Dir::open(
        &source, nix::fcntl::OFlag::O_NOFOLLOW, nix::sys::stat::Mode::empty())?;

    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o640)
        .open(archive)?;

    let mut writer = std::io::BufWriter::with_capacity(1024*1024, file);
    let mut feature_flags = pxar::CA_FORMAT_DEFAULT;
    if no_xattrs {
        feature_flags ^= pxar::CA_FORMAT_WITH_XATTRS;
    }
    if no_fcaps {
        feature_flags ^= pxar::CA_FORMAT_WITH_FCAPS;
    }

    pxar::Encoder::encode(source, &mut dir, &mut writer, all_file_systems, verbose, feature_flags)?;

    writer.flush()?;

    Ok(Value::Null)
}

fn main() {

    let cmd_def = CliCommandMap::new()
        .insert("create", CliCommand::new(
            ApiMethod::new(
                create_archive,
                ObjectSchema::new("Create new .pxar archive.")
                    .required("archive", StringSchema::new("Archive name"))
                    .required("source", StringSchema::new("Source directory."))
                    .optional("verbose", BooleanSchema::new("Verbose output.").default(false))
                    .optional("no-xattrs", BooleanSchema::new("Ignore extended file attributes.").default(false))
                    .optional("no-fcaps", BooleanSchema::new("Ignore file capabilities.").default(false))
                    .optional("all-file-systems", BooleanSchema::new("Include mounted sudirs.").default(false))
           ))
            .arg_param(vec!["archive", "source"])
            .completion_cb("archive", tools::complete_file_name)
            .completion_cb("source", tools::complete_file_name)
           .into()
        )
        .insert("extract", CliCommand::new(
            ApiMethod::new(
                extract_archive,
                ObjectSchema::new("Extract an archive.")
                    .required("archive", StringSchema::new("Archive name."))
                    .required("target", StringSchema::new("Target directory."))
                    .optional("verbose", BooleanSchema::new("Verbose output.").default(false))
                    .optional("no-xattrs", BooleanSchema::new("Ignore extended file attributes.").default(false))
                    .optional("no-fcaps", BooleanSchema::new("Ignore file capabilities.").default(false))
          ))
            .arg_param(vec!["archive", "target"])
            .completion_cb("archive", tools::complete_file_name)
            .completion_cb("target", tools::complete_file_name)
            .into()
        )
        .insert("list", CliCommand::new(
            ApiMethod::new(
                print_filenames,
                ObjectSchema::new("List the contents of an archive.")
                    .required("archive", StringSchema::new("Archive name."))
            ))
            .arg_param(vec!["archive"])
            .completion_cb("archive", tools::complete_file_name)
            .into()
        )
        .insert("dump", CliCommand::new(
            ApiMethod::new(
                dump_archive,
                ObjectSchema::new("Textual dump of archive contents (debug toolkit).")
                    .required("archive", StringSchema::new("Archive name."))
            ))
            .arg_param(vec!["archive"])
            .completion_cb("archive", tools::complete_file_name)
            .into()
        );

    run_cli_command(cmd_def.into());
}
