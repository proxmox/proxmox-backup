use anyhow::{Error};

use proxmox_backup::api2::types::Userid;
use proxmox_backup::client::*;

async fn upload_speed() -> Result<f64, Error> {

    let host = "localhost";
    let datastore = "store2";

    let username = Userid::root_userid();

    let options = HttpClientOptions::new()
        .interactive(true)
        .ticket_cache(true);

    let client = HttpClient::new(host, username, options)?;

    let backup_time = chrono::Utc::now();

    let client = BackupWriter::start(client, None, datastore, "host", "speedtest", backup_time, false).await?;

    println!("start upload speed test");
    let res = client.upload_speedtest(true).await?;

    Ok(res)
}

fn main()  {
    match proxmox_backup::tools::runtime::main(upload_speed()) {
        Ok(mbs) => {
            println!("average upload speed: {} MB/s", mbs);
        }
        Err(err) => {
            eprintln!("ERROR: {}", err);
        }
    }
}
