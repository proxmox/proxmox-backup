use failure::*;
use futures::*;

use serde_json::Value;
use proxmox_backup::client::*;

fn get(mut h2: h2::client::SendRequest<bytes::Bytes>, path: &str) -> impl Future<Item=Value, Error=Error> {

    let request = http::Request::builder()
        .method("GET")
        .uri(format!("https://localhost/{}", path))
        .header(hyper::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(()).unwrap();

    println!("SEND GET {} REQUEST", path);
    let (response, _stream) = h2.send_request(request, true).unwrap();

    response
        .map_err(Error::from)
        .and_then(|response| {
            let (head, mut body) = response.into_parts();

            println!("Received response: {:?}", head);

            // The `release_capacity` handle allows the caller to manage
            // flow control.
            //
            // Whenever data is received, the caller is responsible for
            // releasing capacity back to the server once it has freed
            // the data from memory.
            let mut release_capacity = body.release_capacity().clone();

            body
                .concat2()
                .map_err(Error::from)
                .and_then(move |data| {
                    println!("RX: {:?}", data);

                    // fixme:
                    Ok(Value::Null)
                })
        }).map_err(Error::from)
}

fn run() -> Result<(), Error> {

    let host = "localhost";

    let username = "root@pam";

    let mut client = HttpClient::new(host, username)?;

    let h2client = client.h2upgrade("/api2/json/admin/datastore/store2/h2upload");

    let res = h2client.and_then(|mut h2| {
        println!("start http2");

        let result1 = get(h2.clone(), "test1");
        let result2 = get(h2.clone(), "test2");

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
