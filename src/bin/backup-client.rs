extern crate proxmox_backup;

use failure::*;
use std::os::unix::io::AsRawFd;

use proxmox_backup::tools;
use proxmox_backup::cli::command::*;
use proxmox_backup::api::schema::*;
use proxmox_backup::api::router::*;
//use proxmox_backup::backup::chunk_store::*;
//use proxmox_backup::backup::image_index::*;
//use proxmox_backup::config::datastore;
use proxmox_backup::backup::datastore::*;
use serde_json::{Value};

fn required_string_param<'a>(param: &'a Value, name: &str) -> &'a str {
    param[name].as_str().expect(&format!("missing parameter '{}'", name))
}


fn backup_file(param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    let filename = required_string_param(&param, "filename");
    let store = required_string_param(&param, "store");
    let target = required_string_param(&param, "target");

    let mut datastore = DataStore::open(store)?;

    println!("Backup file '{}' to '{}'", filename, store);

    let mut target = std::path::PathBuf::from(target);
    if let Some(ext) = target.extension() {
        if ext != "iidx" {
            bail!("got wrong file extension - expected '.iidx'");
        }
    } else {
        target.set_extension("iidx");
    }

    {
        let file = std::fs::File::open(filename)?;
        let stat = nix::sys::stat::fstat(file.as_raw_fd())?;
        if stat.st_size <= 0 { bail!("got strange file size '{}'", stat.st_size); }
        let size = stat.st_size as usize;

        let mut index = datastore.create_image_writer(&target, size)?;

        tools::file_chunker(file, 64*1024, |pos, chunk| {
            index.add_chunk(pos, chunk)?;
            Ok(true)
        })?;

        index.close()?; // commit changes
    }

    datastore.garbage_collection()?;

    let idx = datastore.open_image_reader(target)?;
    idx.print_info();

    Ok(Value::Null)
}


fn main() {

    let cmd_def = CliCommand::new(
        ApiMethod::new(
            backup_file,
            ObjectSchema::new("Create backup from file.")
                .required("filename", StringSchema::new("Source file name."))
                .required("store", StringSchema::new("Datastore name."))
                .required("target", StringSchema::new("Target name."))
        ))
        .arg_param(vec!["filename", "target"])
        .completion_cb("store", proxmox_backup::config::datastore::complete_datastore_name);


    if let Err(err) = run_cli_command(&cmd_def.into()) {
        eprintln!("Error: {}", err);
        print_cli_usage();
        std::process::exit(-1);
    }

}
