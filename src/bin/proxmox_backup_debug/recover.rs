use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{bail, format_err, Error};

use proxmox_router::cli::{CliCommand, CliCommandMap, CommandLineInterface};
use proxmox_schema::api;

use pbs_client::tools::key_source::get_encryption_key_password;
use pbs_datastore::dynamic_index::DynamicIndexReader;
use pbs_datastore::file_formats::{DYNAMIC_SIZED_CHUNK_INDEX_1_0, FIXED_SIZED_CHUNK_INDEX_1_0};
use pbs_datastore::fixed_index::FixedIndexReader;
use pbs_datastore::index::IndexFile;
use pbs_datastore::DataBlob;
use pbs_key_config::load_and_decrypt_key;
use pbs_tools::crypt_config::CryptConfig;

#[api(
    input: {
        properties: {
            file: {
                description: "Path to the index file, either .fidx or .didx.",
                type: String,
            },
            chunks: {
                description: "Path to the directory that contains the chunks, usually <datastore>/.chunks.",
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
            },
            "ignore-missing-chunks": {
                description: "If a chunk is missing, warn and write 0-bytes instead to attempt partial recovery.",
                type: Boolean,
                optional: true,
                default: false,
            },
            "ignore-corrupt-chunks": {
                description: "If a chunk is corrupt, warn and write 0-bytes instead to attempt partial recovery.",
                type: Boolean,
                optional: true,
                default: false,
            },
            "output-path": {
                type: String,
                description: "Output file path, defaults to `file` without extension, '-' means STDOUT.",
                optional: true,
            },
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
    ignore_missing_chunks: bool,
    ignore_corrupt_chunks: bool,
    output_path: Option<String>,
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

    let output_path = output_path.unwrap_or_else(|| {
        let filename = file_path.file_stem().unwrap().to_str().unwrap();
        filename.to_string()
    });

    let output_path = match output_path.as_str() {
        "-" => None,
        path => Some(path),
    };
    let mut output_file = crate::outfile_or_stdout(output_path)
        .map_err(|e| format_err!("could not create output file - {}", e))?;

    let mut data = Vec::with_capacity(4 * 1024 * 1024);
    for pos in 0..index.index_count() {
        let chunk_digest = index.index_digest(pos).unwrap();
        let digest_str = hex::encode(chunk_digest);
        let digest_prefix = &digest_str[0..4];
        let chunk_path = chunks_path.join(digest_prefix).join(digest_str);

        let create_zero_chunk = |msg: String| -> Result<(DataBlob, Option<&[u8; 32]>), Error> {
            let info = index
                .chunk_info(pos)
                .ok_or_else(|| format_err!("Couldn't read chunk info from index at {pos}"))?;
            let size = info.size();

            eprintln!("WARN: chunk {:?} {}", chunk_path, msg);
            eprintln!("WARN: replacing output file {:?} with '\\0'", info.range,);

            Ok((
                DataBlob::encode(&vec![0; size as usize], crypt_conf_opt.as_ref(), true)?,
                None,
            ))
        };

        let (chunk_blob, chunk_digest) = match std::fs::File::open(&chunk_path) {
            Ok(mut chunk_file) => {
                data.clear();
                chunk_file.read_to_end(&mut data)?;

                // first chance for corrupt chunk - handling magic fails
                DataBlob::from_raw(data.clone())
                    .map(|blob| (blob, Some(chunk_digest)))
                    .or_else(|err| {
                        if ignore_corrupt_chunks {
                            create_zero_chunk(format!("is corrupt - {err}"))
                        } else {
                            bail!("Failed to parse chunk {chunk_path:?} - {err}");
                        }
                    })?
            }
            Err(err) => {
                if ignore_missing_chunks && err.kind() == std::io::ErrorKind::NotFound {
                    create_zero_chunk("is missing".to_string())?
                } else {
                    bail!("could not open chunk file - {}", err);
                }
            }
        };

        // second chance - we need CRC to detect truncated chunks!
        let crc_res = if skip_crc {
            Ok(())
        } else {
            chunk_blob.verify_crc()
        };

        let (chunk_blob, chunk_digest) = if let Err(crc_err) = crc_res {
            if ignore_corrupt_chunks {
                create_zero_chunk(format!("is corrupt - {crc_err}"))?
            } else {
                bail!("Error at chunk {:?} - {crc_err}", chunk_path);
            }
        } else {
            (chunk_blob, chunk_digest)
        };

        // third chance - decoding might fail (digest, compression, encryption)
        let decoded = chunk_blob
            .decode(crypt_conf_opt.as_ref(), chunk_digest)
            .or_else(|err| {
                if ignore_corrupt_chunks {
                    create_zero_chunk(format!("fails to decode - {err}"))?
                        .0
                        .decode(crypt_conf_opt.as_ref(), None)
                } else {
                    bail!("Failed to decode chunk {:?} = {}", chunk_path, err);
                }
            })?;

        output_file.write_all(decoded.as_slice())?;
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
