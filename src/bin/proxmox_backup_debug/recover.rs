use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{bail, format_err, Error};
use serde_json::Value;

use proxmox_router::cli::{CliCommand, CliCommandMap, CommandLineInterface};
use proxmox_schema::api;

use pbs_tools::crypt_config::CryptConfig;
use pbs_datastore::dynamic_index::DynamicIndexReader;
use pbs_datastore::file_formats::{DYNAMIC_SIZED_CHUNK_INDEX_1_0, FIXED_SIZED_CHUNK_INDEX_1_0};
use pbs_datastore::fixed_index::FixedIndexReader;
use pbs_datastore::index::IndexFile;
use pbs_datastore::DataBlob;
use pbs_config::key_config::load_and_decrypt_key;
use pbs_client::tools::key_source::get_encryption_key_password;

#[api(
    input: {
        properties: {
            file: {
                description: "Path to the index file, either .fidx or .didx.",
                type: String,
            },
            chunks: {
                description: "Path to the directorty that contains the chunks, usually <datastore>/.chunks.",
                type: String,
            },
            "keyfile": {
                description: "Path to a keyfile, if the data was encrypted, a keyfile is needed for decryption.",
                type: String,
                optional: true,
            },
            "skip-crc": {
                description: "Skip the crc verification, increases the restore speed by lot.",
                type: Boolean,
                optional: true,
                default: false,
            }
        }
    }
)]
/// Restore the data from an index file, given the directory of where chunks
/// are saved, the index file and a keyfile, if needed for decryption.
fn recover_index(
    file: String,
    chunks: String,
    keyfile: Option<String>,
    skip_crc: bool,
    _param: Value,
) -> Result<(), Error> {
    let file_path = Path::new(&file);
    let chunks_path = Path::new(&chunks);

    let key_file_path = keyfile.as_ref().map(Path::new);

    let mut file = File::open(Path::new(&file))?;
    let mut magic = [0; 8];
    file.read_exact(&mut magic)?;
    file.seek(SeekFrom::Start(0))?;
    let index: Box<dyn IndexFile> = match magic {
        FIXED_SIZED_CHUNK_INDEX_1_0 => Box::new(FixedIndexReader::new(file)?) as Box<dyn IndexFile>,
        DYNAMIC_SIZED_CHUNK_INDEX_1_0 => {
            Box::new(DynamicIndexReader::new(file)?) as Box<dyn IndexFile>
        }
        _ => bail!(format_err!(
            "index file must either be a .fidx or a .didx file"
        )),
    };

    let crypt_conf_opt = if let Some(key_file_path) = key_file_path {
        let (key, _created, _fingerprint) =
            load_and_decrypt_key(key_file_path, &get_encryption_key_password)?;
        Some(CryptConfig::new(key)?)
    } else {
        None
    };

    let output_filename = file_path.file_stem().unwrap().to_str().unwrap();
    let output_path = Path::new(output_filename);
    let mut output_file = File::create(output_path)
        .map_err(|e| format_err!("could not create output file - {}", e))?;

    let mut data = Vec::with_capacity(4 * 1024 * 1024);
    for pos in 0..index.index_count() {
        let chunk_digest = index.index_digest(pos).unwrap();
        let digest_str = hex::encode(chunk_digest);
        let digest_prefix = &digest_str[0..4];
        let chunk_path = chunks_path.join(digest_prefix).join(digest_str);
        let mut chunk_file = std::fs::File::open(&chunk_path)
            .map_err(|e| format_err!("could not open chunk file - {}", e))?;

        data.clear();
        chunk_file.read_to_end(&mut data)?;
        let chunk_blob = DataBlob::from_raw(data.clone())?;

        if !skip_crc {
            chunk_blob.verify_crc()?;
        }

        output_file.write_all(
            chunk_blob
                .decode(crypt_conf_opt.as_ref(), Some(chunk_digest))?
                .as_slice(),
        )?;
    }

    Ok(())
}

pub fn recover_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new().insert(
        "index",
        CliCommand::new(&API_METHOD_RECOVER_INDEX).arg_param(&["file", "chunks"]),
    );
    cmd_def.into()
}
