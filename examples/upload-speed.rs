use anyhow::{Error};

use proxmox_backup::client::*;

async fn upload_speed() -> Result<usize, Error> {

    let host = "localhost";
    let datastore = "store2";

    let username = "root@pam";

    let options = HttpClientOptions::new()
        .interactive(true)
        .ticket_cache(true);

    let client = HttpClient::new(host, username, options)?;

    let backup_time = chrono::Utc::now();

    let client = BackupWriter::start(client, None, datastore, "host", "speedtest", backup_time, false).await?;

    println!("start upload speed test");
    let res = client.upload_speedtest().await?;

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
