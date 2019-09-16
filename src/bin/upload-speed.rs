use failure::*;

use proxmox_backup::client::*;

async fn upload_speed() -> Result<usize, Error> {

    let host = "localhost";
    let datastore = "store2";

    let username = "root@pam";

    let client = HttpClient::new(host, username, None)?;

    let backup_time = chrono::Utc::now();

    let client = client.start_backup(datastore, "host", "speedtest", backup_time, false).await?;

    println!("start upload speed test");
    let res = client.upload_speedtest().await?;

    Ok(res)
}

#[tokio::main]
async fn main()  {
    match upload_speed().await {
        Ok(mbs) => {
            println!("average upload speed: {} MB/s", mbs);
        }
        Err(err) => {
            eprintln!("ERROR: {}", err);
        }
    }
}
