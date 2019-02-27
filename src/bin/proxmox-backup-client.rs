extern crate proxmox_backup;

use failure::*;
//use std::os::unix::io::AsRawFd;
use chrono::{Local, TimeZone};

use proxmox_backup::tools;
use proxmox_backup::cli::*;
use proxmox_backup::api_schema::*;
use proxmox_backup::api_schema::router::*;
use proxmox_backup::client::*;
use proxmox_backup::backup::*;
//use proxmox_backup::backup::image_index::*;
//use proxmox_backup::config::datastore;
//use proxmox_backup::catar::encoder::*;
//use proxmox_backup::backup::datastore::*;

use serde_json::{Value};
use hyper::Body;
use std::sync::Arc;
use regex::Regex;

use lazy_static::lazy_static;

lazy_static! {
    static ref BACKUPSPEC_REGEX: Regex = Regex::new(r"^([a-zA-Z0-9_-]+):(.+)$").unwrap();
}

fn backup_directory(
    client: &mut HttpClient,
    repo: &BackupRepository,
    body: Body,
    archive_name: &str,
    backup_time: u64,
    chunk_size: Option<u64>,
) -> Result<(), Error> {

    let mut query = url::form_urlencoded::Serializer::new(String::new());

    query
        .append_pair("archive_name", archive_name)
        .append_pair("type", "host")
        .append_pair("id", &tools::nodename())
        .append_pair("time", &backup_time.to_string());

    if let Some(size) = chunk_size {
        query.append_pair("chunk-size", &size.to_string());
    }

    let query = query.finish();

    let path = format!("api2/json/admin/datastore/{}/catar?{}", repo.store, query);

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

    let repo_url = tools::required_string_param(&param, "repository")?;
    let repo = BackupRepository::parse(repo_url)?;

    let mut client = HttpClient::new(&repo.host, &repo.user);

    let path = format!("api2/json/admin/datastore/{}/backups", repo.store);

    let result = client.get(&path)?;

    // fixme: implement and use output formatter instead ..
    let list = result["data"].as_array().unwrap();

    for item in list {

        let id = item["backup_id"].as_str(). unwrap();
        let btype = item["backup_type"].as_str(). unwrap();
        let epoch = item["backup_time"].as_i64(). unwrap();

        let time_str = Local.timestamp(epoch, 0).format("%c");

        let files = item["files"].as_array().unwrap();

        for file in files {
            let filename = file.as_str().unwrap();
            println!("| {} | {} | {} | {}", btype, id, time_str, filename);
        }
    }

    //Ok(result)
    Ok(Value::Null)
}

fn start_garbage_collection(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;
    let repo = BackupRepository::parse(repo_url)?;

    let mut client = HttpClient::new(&repo.host, &repo.user);

    let path = format!("api2/json/admin/datastore/{}/gc", repo.store);

    let result = client.post(&path)?;

    Ok(result)
}

fn parse_backupspec(value: &str) -> Result<(&str, &str), Error> {

    if let Some(caps) = BACKUPSPEC_REGEX.captures(value) {
        return Ok((caps.get(1).unwrap().as_str(), caps.get(2).unwrap().as_str()));
    }
    bail!("unable to parse directory specification '{}'", value);
}

fn create_backup(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let repo_url = tools::required_string_param(&param, "repository")?;

    let backupspec_list = tools::required_array_param(&param, "backupspec")?;

    let repo = BackupRepository::parse(repo_url)?;

    let chunk_size_opt = param["chunk-size"].as_u64().map(|v| v*1024);

    if let Some(size) = chunk_size_opt {
        verify_chunk_size(size)?;
    }

    let mut upload_list = vec![];

    for backupspec in backupspec_list {
        let (target, filename) = parse_backupspec(backupspec.as_str().unwrap())?;

        let stat = match nix::sys::stat::stat(filename) {
            Ok(s) => s,
            Err(err) => bail!("unable to access '{}' - {}", filename, err),
        };

        if (stat.st_mode & libc::S_IFDIR) != 0 {
             let stream = CaTarBackupStream::open(filename)?;

            let body = Body::wrap_stream(stream);

            let target = format!("{}.catar", target);

            upload_list.push((body, filename.to_owned(), target));

        } else if (stat.st_mode & (libc::S_IFREG|libc::S_IFBLK)) != 0 {
            if stat.st_size <= 0 { bail!("got strange file size '{}'", stat.st_size); }
            let _size = stat.st_size as usize;

            panic!("implement me");

            //backup_image(&datastore, &file, size, &target, chunk_size)?;

            // let idx = datastore.open_image_reader(target)?;
            // idx.print_info();

        } else {
            bail!("unsupported file type (expected a directory, file or block device)");
        }
    }

    let backup_time = std::time::SystemTime::now().duration_since(
        std::time::SystemTime::UNIX_EPOCH)?.as_secs();

    let mut client = HttpClient::new(&repo.host, &repo.user);

    for (body, filename, target) in upload_list {
        println!("Upload '{}' to '{:?}'", filename, repo);
        backup_directory(&mut client, &repo, body, &target, backup_time, chunk_size_opt)?;
    }

    //datastore.garbage_collection()?;

    Ok(Value::Null)
}

fn main() {

    let repo_url_schema: Arc<Schema> = Arc::new(
        StringSchema::new("Repository URL.")
            .format(BACKUP_REPO_URL.clone())
            .max_length(256)
            .into()
    );

    let backup_source_schema: Arc<Schema> = Arc::new(
        StringSchema::new("Backup source specification ([<label>:<path>]).")
            .format(Arc::new(ApiStringFormat::Pattern(&BACKUPSPEC_REGEX)))
            .into()
    );

    let create_cmd_def = CliCommand::new(
        ApiMethod::new(
            create_backup,
            ObjectSchema::new("Create backup.")
                .required("repository", repo_url_schema.clone())
                .required(
                    "backupspec",
                    ArraySchema::new(
                        "List of backup source specifications ([<label>:<path>] ...)",
                        backup_source_schema,
                    ).min_length(1)
                )
                .optional(
                    "chunk-size",
                    IntegerSchema::new("Chunk size in KB. Must be a power of 2.")
                        .minimum(64)
                        .maximum(4096)
                        .default(4096)
                )
        ))
        .arg_param(vec!["repository", "backupspec"])
        .completion_cb("backupspec", tools::complete_file_name);

    let list_cmd_def = CliCommand::new(
        ApiMethod::new(
            list_backups,
            ObjectSchema::new("List backups.")
                .required("repository", repo_url_schema.clone())
        ))
        .arg_param(vec!["repository"]);

    let garbage_collect_cmd_def = CliCommand::new(
        ApiMethod::new(
            start_garbage_collection,
            ObjectSchema::new("Start garbage collection for a specific repository.")
                .required("repository", repo_url_schema.clone())
        ))
        .arg_param(vec!["repository"]);

    let cmd_def = CliCommandMap::new()
        .insert("create".to_owned(), create_cmd_def.into())
        .insert("garbage-collect".to_owned(), garbage_collect_cmd_def.into())
        .insert("list".to_owned(), list_cmd_def.into());

    run_cli_command(cmd_def.into());
}
