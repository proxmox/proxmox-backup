use failure::*;
use futures::*;
use serde_json::json;

use proxmox_backup::client::*;

fn upload_speed() -> Result<usize, Error> {

    let host = "localhost";
    let datastore = "store2";

    let username = "root@pam";

    let mut client = HttpClient::new(host, username)?;

    let upgrade = client.start_backup(datastore, "host", "speedtest");

    let res = upgrade.and_then(|h2| {
        println!("start upload speed test");
        h2.upload_speedtest()
    }).wait()?;

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
