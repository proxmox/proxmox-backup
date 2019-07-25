use failure::*;
use futures::*;

use proxmox_backup::client::*;

fn upload_speed() -> Result<usize, Error> {

    let host = "localhost";
    let datastore = "store2";

    let username = "root@pam";

    let client = HttpClient::new(host, username)?;

    let backup_time = chrono::Utc::now();

    let client = client.start_backup(datastore, "host", "speedtest", backup_time, false).wait()?;

    println!("start upload speed test");
    let res = client.upload_speedtest().wait()?;

    Ok(res)
}

fn main()  {

    let mut rt = tokio::runtime::Runtime::new().unwrap();

    // should be rt.block_on_all, but this block forever in release builds
    let _ = rt.block_on(futures::future::lazy(move || -> Result<(), ()> {
        match upload_speed() {
            Ok(mbs) => {
                println!("average upload speed: {} MB/s", mbs);
            }
            Err(err) => {
                eprintln!("ERROR: {}", err);
            }
        }
        Ok(())
    }));
}
