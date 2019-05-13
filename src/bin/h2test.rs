use failure::*;
use futures::*;

use serde_json::json;
use proxmox_backup::client::*;

fn run() -> Result<(), Error> {

    let host = "localhost";

    let username = "root@pam";

    let mut client = HttpClient::new(host, username)?;

    let param = json!({"backup-type": "host", "backup-id": "test" });
    let upgrade = client.h2upgrade("/api2/json/admin/datastore/store2/backup", Some(param));

    let res = upgrade.and_then(|send_request| {
        println!("start http2");
        let h2 = H2Client::new(send_request);
        let result1 = h2.get("test1", None);
        let result2 = h2.get("test2", None);

        result1.join(result2)
    }).wait()?;

    println!("GOT {:?}", res);

    Ok(())
}

fn main()  {

    hyper::rt::run(futures::future::lazy(move || {
        if let Err(err) = run() {
            eprintln!("ERROR: {}", err);
        }
        Ok(())
    }));
}
