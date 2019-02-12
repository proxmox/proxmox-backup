extern crate proxmox_backup;

use failure::*;
//use std::os::unix::io::AsRawFd;

use proxmox_backup::tools;
use proxmox_backup::cli::command::*;
use proxmox_backup::api::schema::*;
use proxmox_backup::api::router::*;
use proxmox_backup::client::http_client::*;
use proxmox_backup::client::catar_backup_stream::*;
//use proxmox_backup::backup::chunk_store::*;
//use proxmox_backup::backup::image_index::*;
//use proxmox_backup::config::datastore;
//use proxmox_backup::catar::encoder::*;
//use proxmox_backup::backup::datastore::*;

use serde_json::{Value};
use hyper::Body;

fn backup_directory(body: Body, store: &str, archive_name: &str) -> Result<(), Error> {

    let client = HttpClient::new("localhost");

    let epoch = std::time::SystemTime::now().duration_since(
        std::time::SystemTime::UNIX_EPOCH)?.as_secs();

    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("archive_name", archive_name)
        .append_pair("type", "host")
        .append_pair("id", &tools::nodename())
        .append_pair("time", &epoch.to_string())
        .finish();

    let path = format!("api2/json/admin/datastore/{}/catar?{}", store, query);

    client.upload("application/x-proxmox-backup-catar", body, &path)?;

    Ok(())
}

/****
fn backup_image(datastore: &DataStore, file: &std::fs::File, size: usize, target: &str, chunk_size: usize) -> Result<(), Error> {

    let mut target = PathBuf::from(target);

    if let Some(ext) = target.extension() {
        if ext != "fidx" {
            bail!("got wrong file extension - expected '.fidx'");
        }
    } else {
        target.set_extension("fidx");
    }

    let mut index = datastore.create_image_writer(&target, size, chunk_size)?;

    tools::file_chunker(file, chunk_size, |pos, chunk| {
        index.add_chunk(pos, chunk)?;
        Ok(true)
    })?;

    index.close()?; // commit changes

    Ok(())
}
*/

fn list_backups(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let store = tools::required_string_param(&param, "store")?;

    let client = HttpClient::new("localhost");

    let path = format!("api2/json/admin/datastore/{}/backups", store);

    let result = client.get(&path)?;

    Ok(result)
}

fn create_backup(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let filename = tools::required_string_param(&param, "filename")?;
    let store = tools::required_string_param(&param, "store")?;
    let target = tools::required_string_param(&param, "target")?;

    let mut _chunk_size = 4*1024*1024;

    if let Some(size) = param["chunk-size"].as_u64() {
        static SIZES: [u64; 7] = [64, 128, 256, 512, 1024, 2048, 4096];

        if SIZES.contains(&size) {
            _chunk_size = (size as usize) * 1024;
        } else {
            bail!("Got unsupported chunk size '{}'", size);
        }
    }

    let stat = match nix::sys::stat::stat(filename) {
        Ok(s) => s,
        Err(err) => bail!("unable to access '{}' - {}", filename, err),
    };

    if (stat.st_mode & libc::S_IFDIR) != 0 {
        println!("Backup directory '{}' to '{}'", filename, store);

        let stream = CaTarBackupStream::open(filename)?;

        let body = Body::wrap_stream(stream);

        backup_directory(body, store, target)?;

    } else if (stat.st_mode & (libc::S_IFREG|libc::S_IFBLK)) != 0 {
        println!("Backup image '{}' to '{}'", filename, store);

        if stat.st_size <= 0 { bail!("got strange file size '{}'", stat.st_size); }
        let _size = stat.st_size as usize;

        panic!("implement me");

        //backup_image(&datastore, &file, size, &target, chunk_size)?;

       // let idx = datastore.open_image_reader(target)?;
       // idx.print_info();

    } else {
        bail!("unsupported file type (expected a directory, file or block device)");
    }

    //datastore.garbage_collection()?;

    Ok(Value::Null)
}

fn main() {

    let create_cmd_def = CliCommand::new(
        ApiMethod::new(
            create_backup,
            ObjectSchema::new("Create backup.")
                .required("filename", StringSchema::new("Source name (file or directory name)"))
                .required("store", StringSchema::new("Datastore name."))
                .required("target", StringSchema::new("Target name."))
                .optional(
                    "chunk-size",
                    IntegerSchema::new("Chunk size in KB. Must be a power of 2.")
                        .minimum(64)
                        .maximum(4096)
                        .default(4096)
                )
        ))
        .arg_param(vec!["filename", "target"])
        .completion_cb("filename", tools::complete_file_name)
        .completion_cb("store", proxmox_backup::config::datastore::complete_datastore_name);

    let list_cmd_def = CliCommand::new(
        ApiMethod::new(
            list_backups,
            ObjectSchema::new("List backups.")
                .required("store", StringSchema::new("Datastore name."))
        ))
        .arg_param(vec!["store"])
        .completion_cb("store", proxmox_backup::config::datastore::complete_datastore_name);


    let cmd_def = CliCommandMap::new()
        .insert("create".to_owned(), create_cmd_def.into())
        .insert("list".to_owned(), list_cmd_def.into());

    if let Err(err) = run_cli_command(&cmd_def.into()) {
        eprintln!("Error: {}", err);
        if err.downcast::<UsageError>().is_ok() {
            print_cli_usage();
        }
        std::process::exit(-1);
    }

}
