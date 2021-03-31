use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Error};
use serde_json::Value;
use serde::Serialize;

use proxmox::api::{ApiMethod, RpcEnvironment};
use proxmox::api::{
    api,
    cli::{
        OUTPUT_FORMAT,
        ColumnConfig,
        get_output_format,
        format_and_print_result_full,
        default_table_format_options,
    },
    router::ReturnType,
};

use proxmox_backup::backup::{
    load_and_decrypt_key,
    CryptConfig,
    KeyDerivationConfig,
    DataChunkBuilder,
};

use proxmox_backup::client::*;

use crate::{
    KEYFILE_SCHEMA, REPO_URL_SCHEMA,
    extract_repository_from_value,
    record_repository,
    connect,
};

use crate::proxmox_client_tools::key_source::get_encryption_key_password;

#[api()]
#[derive(Copy, Clone, Serialize)]
/// Speed test result
struct Speed {
    /// The meassured speed in Bytes/second
    #[serde(skip_serializing_if="Option::is_none")]
    speed: Option<f64>,
    /// Top result we want to compare with
    top: f64,
}

#[api(
    properties: {
        "tls": {
            type: Speed,
        },
        "sha256": {
            type: Speed,
        },
        "compress": {
            type: Speed,
        },
        "decompress": {
            type: Speed,
        },
        "aes256_gcm": {
            type: Speed,
        },
        "verify": {
            type: Speed,
        },
    },
)]
#[derive(Copy, Clone, Serialize)]
/// Benchmark Results
struct BenchmarkResult {
    /// TLS upload speed
    tls: Speed,
    /// SHA256 checksum computation speed
    sha256: Speed,
    /// ZStd level 1 compression speed
    compress: Speed,
    /// ZStd level 1 decompression speed
    decompress: Speed,
    /// AES256 GCM encryption speed
    aes256_gcm: Speed,
    /// Verify speed
    verify: Speed,
}

static BENCHMARK_RESULT_2020_TOP: BenchmarkResult =  BenchmarkResult {
    tls: Speed {
        speed: None,
        top: 1_000_000.0 * 1235.0, // TLS to localhost, AMD Ryzen 7 2700X
    },
    sha256: Speed {
        speed: None,
        top: 1_000_000.0 * 2022.0, // AMD Ryzen 7 2700X
    },
    compress: Speed {
        speed: None,
        top: 1_000_000.0 * 752.0, // AMD Ryzen 7 2700X
    },
    decompress: Speed {
        speed: None,
        top: 1_000_000.0 * 1198.0, // AMD Ryzen 7 2700X
    },
    aes256_gcm: Speed {
        speed: None,
        top: 1_000_000.0 * 3645.0, // AMD Ryzen 7 2700X
    },
    verify: Speed {
        speed: None,
        top: 1_000_000.0 * 758.0, // AMD Ryzen 7 2700X
    },
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
           "output-format": {
               schema: OUTPUT_FORMAT,
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

    let repo = extract_repository_from_value(&param).ok();

    let keyfile = param["keyfile"].as_str().map(PathBuf::from);

    let verbose = param["verbose"].as_bool().unwrap_or(false);

    let output_format = get_output_format(&param);

    let crypt_config = match keyfile {
        None => None,
        Some(path) => {
            let (key, _, _) = load_and_decrypt_key(&path, &get_encryption_key_password)?;
            let crypt_config = CryptConfig::new(key)?;
            Some(Arc::new(crypt_config))
        }
    };

    let mut benchmark_result = BENCHMARK_RESULT_2020_TOP;

    // do repo tests first, because this may prompt for a password
    if let Some(repo) = repo {
        test_upload_speed(&mut benchmark_result, repo, crypt_config.clone(), verbose).await?;
    }

    test_crypt_speed(&mut benchmark_result, verbose)?;

    render_result(&output_format, &benchmark_result)?;

    Ok(())
}

// print comparison table
fn render_result(
    output_format: &str,
    benchmark_result: &BenchmarkResult,
) -> Result<(), Error> {

    let mut data = serde_json::to_value(benchmark_result)?;
    let return_type = ReturnType::new(false, &BenchmarkResult::API_SCHEMA);

    let render_speed = |value: &Value, _record: &Value| -> Result<String, Error> {
        match value["speed"].as_f64() {
            None => Ok(String::from("not tested")),
            Some(speed) => {
                let top = value["top"].as_f64().unwrap();
                Ok(format!("{:.2} MB/s ({:.0}%)", speed/1_000_000.0, (speed*100.0)/top))
            }
        }
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("tls")
                .header("TLS (maximal backup upload speed)")
                .right_align(false).renderer(render_speed))
        .column(ColumnConfig::new("sha256")
                .header("SHA256 checksum computation speed")
                .right_align(false).renderer(render_speed))
        .column(ColumnConfig::new("compress")
                .header("ZStd level 1 compression speed")
                .right_align(false).renderer(render_speed))
        .column(ColumnConfig::new("decompress")
                .header("ZStd level 1 decompression speed")
                .right_align(false).renderer(render_speed))
        .column(ColumnConfig::new("verify")
                .header("Chunk verification speed")
                .right_align(false).renderer(render_speed))
       .column(ColumnConfig::new("aes256_gcm")
                .header("AES256 GCM encryption speed")
                .right_align(false).renderer(render_speed));


    format_and_print_result_full(&mut data, &return_type, output_format, &options);

    Ok(())
}

async fn test_upload_speed(
    benchmark_result: &mut BenchmarkResult,
    repo: BackupRepository,
    crypt_config: Option<Arc<CryptConfig>>,
    verbose: bool,
) -> Result<(), Error> {

    let backup_time = proxmox::tools::time::epoch_i64();

    let client = connect(&repo)?;
    record_repository(&repo);

    if verbose { eprintln!("Connecting to backup server"); }
    let client = BackupWriter::start(
        client,
        crypt_config.clone(),
        repo.store(),
        "host",
        "benchmark",
        backup_time,
        false,
        true
    ).await?;

    if verbose { eprintln!("Start TLS speed test"); }
    let speed = client.upload_speedtest(verbose).await?;

    eprintln!("TLS speed: {:.2} MB/s", speed/1_000_000.0);

    benchmark_result.tls.speed = Some(speed);

    Ok(())
}

// test hash/crypt/compress speed
fn test_crypt_speed(
    benchmark_result: &mut BenchmarkResult,
    _verbose: bool,
) -> Result<(), Error> {

    let pw = b"test";

    let kdf = KeyDerivationConfig::Scrypt {
        n: 65536,
        r: 8,
        p: 1,
        salt: Vec::new(),
    };

    let testkey = kdf.derive_key(pw)?;

    let crypt_config = CryptConfig::new(testkey)?;

    //let random_data = proxmox::sys::linux::random_data(1024*1024)?;
    let mut random_data = vec![];
        // generate pseudo random byte sequence
        for i in 0..256*1024 {
            for j in 0..4 {
                let byte = ((i >> (j<<3))&0xff) as u8;
                random_data.push(byte);
            }
        }

    assert_eq!(random_data.len(), 1024*1024);

    let start_time = std::time::Instant::now();

    let mut bytes = 0;
    loop  {
        openssl::sha::sha256(&random_data);
        bytes += random_data.len();
        if start_time.elapsed().as_micros() > 1_000_000 { break; }
    }
    let speed = (bytes as f64)/start_time.elapsed().as_secs_f64();
    benchmark_result.sha256.speed = Some(speed);

    eprintln!("SHA256 speed: {:.2} MB/s", speed/1_000_000.0);


    let start_time = std::time::Instant::now();

    let mut bytes = 0;
    loop  {
        let mut reader = &random_data[..];
        zstd::stream::encode_all(&mut reader, 1)?;
        bytes += random_data.len();
        if start_time.elapsed().as_micros() > 3_000_000 { break; }
    }
    let speed = (bytes as f64)/start_time.elapsed().as_secs_f64();
    benchmark_result.compress.speed = Some(speed);

    eprintln!("Compression speed: {:.2} MB/s", speed/1_000_000.0);


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
    benchmark_result.decompress.speed = Some(speed);

    eprintln!("Decompress speed: {:.2} MB/s", speed/1_000_000.0);


    let start_time = std::time::Instant::now();

    let mut bytes = 0;
    loop  {
        let mut out = Vec::new();
        crypt_config.encrypt_to(&random_data, &mut out)?;
        bytes += random_data.len();
        if start_time.elapsed().as_micros() > 1_000_000 { break; }
    }
    let speed = (bytes as f64)/start_time.elapsed().as_secs_f64();
    benchmark_result.aes256_gcm.speed = Some(speed);

    eprintln!("AES256/GCM speed: {:.2} MB/s", speed/1_000_000.0);


    let start_time = std::time::Instant::now();

    let (chunk, digest) = DataChunkBuilder::new(&random_data)
        .compress(true)
        .build()?;

    let mut bytes = 0;
    loop  {
        chunk.verify_unencrypted(random_data.len(), &digest)?;
        bytes += random_data.len();
        if start_time.elapsed().as_micros() > 1_000_000 { break; }
    }
    let speed = (bytes as f64)/start_time.elapsed().as_secs_f64();
    benchmark_result.verify.speed = Some(speed);

    eprintln!("Verify speed: {:.2} MB/s", speed/1_000_000.0);

    Ok(())
}
