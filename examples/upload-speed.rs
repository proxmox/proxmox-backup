use anyhow::Error;

use pbs_api_types::{Authid, BackupNamespace, BackupType};
use pbs_client::{BackupWriter, HttpClient, HttpClientOptions};

async fn upload_speed() -> Result<f64, Error> {
    let host = "localhost";
    let datastore = "store2";

    let auth_id = Authid::root_auth_id();

    let options = HttpClientOptions::default()
        .interactive(true)
        .ticket_cache(true);

    let client = HttpClient::new(host, 8007, auth_id, options)?;

    let backup_time = proxmox_time::epoch_i64();

    let client = BackupWriter::start(
        client,
        None,
        datastore,
        &BackupNamespace::root(),
        &(BackupType::Host, "speedtest".to_string(), backup_time).into(),
        false,
        true,
    )
    .await?;

    println!("start upload speed test");
    let res = client.upload_speedtest().await?;

    Ok(res)
}

fn main() {
    match proxmox_async::runtime::main(upload_speed()) {
        Ok(mbs) => {
            println!("average upload speed: {} MB/s", mbs);
        }
        Err(err) => {
            eprintln!("ERROR: {}", err);
        }
    }
}
