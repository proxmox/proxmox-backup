use std::io::Write;

use anyhow::Error;

use pbs_api_types::{Authid, BackupNamespace, BackupType};
use pbs_client::{BackupReader, HttpClient, HttpClientOptions};

pub struct DummyWriter {
    bytes: usize,
}

impl Write for DummyWriter {
    fn write(&mut self, data: &[u8]) -> Result<usize, std::io::Error> {
        self.bytes += data.len();
        Ok(data.len())
    }

    fn flush(&mut self) -> Result<(), std::io::Error> {
        Ok(())
    }
}

async fn run() -> Result<(), Error> {
    let host = "localhost";

    let auth_id = Authid::root_auth_id();

    let options = HttpClientOptions::default()
        .interactive(true)
        .ticket_cache(true);

    let client = HttpClient::new(host, 8007, auth_id, options)?;

    let backup_time = proxmox_time::parse_rfc3339("2019-06-28T10:49:48Z")?;

    let client = BackupReader::start(
        &client,
        None,
        "store2",
        &BackupNamespace::root(),
        &(BackupType::Host, "elsa".to_string(), backup_time).into(),
        true,
    )
    .await?;

    let start = std::time::SystemTime::now();

    let mut bytes = 0;
    for _ in 0..100 {
        let mut writer = DummyWriter { bytes: 0 };
        client.speedtest(&mut writer).await?;
        println!("Received {} bytes", writer.bytes);
        bytes += writer.bytes;
    }

    let elapsed = start.elapsed().unwrap();
    let elapsed = (elapsed.as_secs() as f64) + (elapsed.subsec_millis() as f64) / 1000.0;

    println!(
        "Downloaded {} bytes, {} MB/s",
        bytes,
        (bytes as f64) / (elapsed * 1024.0 * 1024.0)
    );

    Ok(())
}

fn main() {
    if let Err(err) = proxmox_async::runtime::main(run()) {
        eprintln!("ERROR: {}", err);
    }
    println!("DONE");
}
