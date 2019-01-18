extern crate proxmox_backup;

use failure::*;
use std::os::unix::io::AsRawFd;

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
use proxmox_backup::backup::datastore::*;

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

    let path = format!("api3/json/admin/datastore/{}/upload_catar?{}", store, query);

    client.upload("application/x-proxmox-backup-catar", body, &path)?;

    Ok(())
}

/****
fn backup_image(datastore: &DataStore, file: &std::fs::File, size: usize, target: &str, chunk_size: usize) -> Result<(), Error> {

    let mut target = PathBuf::from(target);

    if let Some(ext) = target.extension() {
        if ext != "iidx" {
            bail!("got wrong file extension - expected '.iidx'");
        }
    } else {
        target.set_extension("iidx");
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

fn create_backup(param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    let filename = tools::required_string_param(&param, "filename")?;
    let store = tools::required_string_param(&param, "store")?;
    let target = tools::required_string_param(&param, "target")?;

    let mut chunk_size = 4*1024*1024;

    if let Some(size) = param["chunk-size"].as_u64() {
        static SIZES: [u64; 7] = [64, 128, 256, 512, 1024, 2048, 4096];

        if SIZES.contains(&size) {
            chunk_size = (size as usize) * 1024;
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
        let size = stat.st_size as usize;

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


pub fn complete_file_name(arg: &str) -> Vec<String> {

    let mut result = vec![];

    use nix::fcntl::OFlag;
    use nix::sys::stat::Mode;
    use nix::fcntl::AtFlags;

    let mut dirname = std::path::PathBuf::from(arg);

    if let Ok(stat) = nix::sys::stat::fstatat(libc::AT_FDCWD, &dirname, AtFlags::empty()) {

    } else {
        if let Some(parent) = dirname.parent() {
            dirname = parent.to_owned();
        }
    }

    let mut dir = match nix::dir::Dir::openat(libc::AT_FDCWD, &dirname, OFlag::O_DIRECTORY, Mode::empty()) {
        Ok(d) => d,
        Err(err) => {
            return result;
        }
    };

    for item in dir.iter() {
        if let Ok(entry) = item {
            if let Ok(name) = entry.file_name().to_str() {
                if name == "." || name == ".." { continue; }
                let mut newpath = dirname.clone();
                newpath.push(name);

                if let Ok(stat) = nix::sys::stat::fstatat(libc::AT_FDCWD, &newpath, AtFlags::empty()) {
                    if (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR {
                        newpath.push("");
                        if let Some(newpath) = newpath.to_str() {
                            result.push(newpath.to_owned());
                        }
                        continue;
                     }
                }
                if let Some(newpath) = newpath.to_str() {
                    result.push(newpath.to_owned());
                }

             }
        }
    }

    result
}

fn main() {

    let cmd_def = CliCommand::new(
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
        .completion_cb("filename", complete_file_name)
        .completion_cb("store", proxmox_backup::config::datastore::complete_datastore_name);


    if let Err(err) = run_cli_command(&cmd_def.into()) {
        eprintln!("Error: {}", err);
        if err.downcast::<UsageError>().is_ok() {
            print_cli_usage();
        }
        std::process::exit(-1);
    }

}
