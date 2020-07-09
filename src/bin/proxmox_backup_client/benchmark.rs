use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Error};
use serde_json::Value;
use chrono::{TimeZone, Utc};

use proxmox::api::{ApiMethod, RpcEnvironment};
use proxmox::api::api;

use proxmox_backup::backup::{
    load_and_decrypt_key,
    CryptConfig,
    KeyDerivationConfig,
};

use proxmox_backup::client::*;

use crate::{
    KEYFILE_SCHEMA, REPO_URL_SCHEMA,
    extract_repository_from_value,
    record_repository,
    connect,
};

#[api(
   input: {
       properties: {
           repository: {
               schema: REPO_URL_SCHEMA,
               optional: true,
           },
           verbose: {
               description: "Verbose output.",
               type: bool,
               optional: true,
           },
           keyfile: {
               schema: KEYFILE_SCHEMA,
               optional: true,
           },
       }
   }
)]
/// Run benchmark tests
pub async fn benchmark(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let repo = extract_repository_from_value(&param)?;

    let keyfile = param["keyfile"].as_str().map(PathBuf::from);

    let verbose = param["verbose"].as_bool().unwrap_or(false);

    let crypt_config = match keyfile {
        None => None,
        Some(path) => {
            let (key, _) = load_and_decrypt_key(&path, &crate::key::get_encryption_key_password)?;
            let crypt_config = CryptConfig::new(key)?;
            Some(Arc::new(crypt_config))
        }
    };

    test_crypt_speed(verbose)?;

    let backup_time = Utc.timestamp(Utc::now().timestamp(), 0);

    let client = connect(repo.host(), repo.user())?;
    record_repository(&repo);

    if verbose { println!("Connecting to backup server"); }
    let client = BackupWriter::start(
        client,
        crypt_config.clone(),
        repo.store(),
        "host",
        "benchmark",
        backup_time,
        false,
    ).await?;

    if verbose { println!("Start upload speed test"); }
    let speed = client.upload_speedtest(verbose).await?;

    println!("Upload speed: {} MiB/s", speed);

    Ok(())
}


// test SHA256 speed
fn test_crypt_speed(verbose: bool) -> Result<(), Error> {

    let pw = b"test";

    let kdf = KeyDerivationConfig::Scrypt {
        n: 65536,
        r: 8,
        p: 1,
        salt: Vec::new(),
    };

    let testkey = kdf.derive_key(pw)?;

    let crypt_config = CryptConfig::new(testkey)?;

    let random_data = proxmox::sys::linux::random_data(1024*1024)?;

    let start_time = std::time::Instant::now();

    let mut bytes = 0;
    loop  {
        openssl::sha::sha256(&random_data);
        bytes += random_data.len();
        if start_time.elapsed().as_micros() > 1_000_000 { break; }
    }
    let speed = (bytes as f64)/start_time.elapsed().as_secs_f64();

    println!("SHA256 speed: {:.2} MB/s", speed/1_000_000_.0);


    let start_time = std::time::Instant::now();

    let mut bytes = 0;
    loop  {
        let mut reader = &random_data[..];
        zstd::stream::encode_all(&mut reader, 1)?;
        bytes += random_data.len();
        if start_time.elapsed().as_micros() > 1_000_000 { break; }
    }
    let speed = (bytes as f64)/start_time.elapsed().as_secs_f64();

    println!("Compression speed: {:.2} MB/s", speed/1_000_000_.0);


    let start_time = std::time::Instant::now();

    let compressed_data = {
        let mut reader = &random_data[..];
        zstd::stream::encode_all(&mut reader, 1)?
    };

    let mut bytes = 0;
    loop  {
        let mut reader = &compressed_data[..];
        let data = zstd::stream::decode_all(&mut reader)?;
        bytes += data.len();
        if start_time.elapsed().as_micros() > 1_000_000 { break; }
    }
    let speed = (bytes as f64)/start_time.elapsed().as_secs_f64();

    println!("Decompress speed: {:.2} MB/s", speed/1_000_000_.0);


    let start_time = std::time::Instant::now();

    let mut bytes = 0;
    loop  {
        let mut out = Vec::new();
        crypt_config.encrypt_to(&random_data, &mut out);
        bytes += random_data.len();
        if start_time.elapsed().as_micros() > 1_000_000 { break; }
    }
    let speed = (bytes as f64)/start_time.elapsed().as_secs_f64();

    println!("AES256/GCM speed: {:.2} MB/s", speed/1_000_000_.0);

    Ok(())
}
