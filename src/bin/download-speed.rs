use failure::*;
use futures::*;
use std::io::Write;

//use std::sync::Arc;
//use serde_json::Value;
use chrono::{DateTime, Local};

//use proxmox_backup::tools;
//use proxmox_backup::backup::*;
use proxmox_backup::client::*;
//use proxmox_backup::pxar;
//use futures::stream::Stream;

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


fn run() -> Result<(), Error> {

    let host = "localhost";

    let username = "root@pam";

    let client = HttpClient::new(host, username)?;

    let backup_time = "2019-06-28T10:49:48+02:00".parse::<DateTime<Local>>()?;

    let client = client.start_backup_reader("store2", "host", "elsa", backup_time, true).wait()?;

    let start = std::time::SystemTime::now();

    futures::stream::repeat(())
        .take(100)
        .and_then(|_| {
            let writer = DummyWriter { bytes: 0 };
            client.speedtest(writer)
                .and_then(|writer| {
                    println!("Received {} bytes", writer.bytes);
                    Ok(writer.bytes)
                })
        })
        .fold(0, move |mut acc, size| {
            acc += size;
            Ok::<_, Error>(acc)
        })
        .then(move |result| {
            match result {
                Err(err) => {
                    println!("ERROR {}", err);
                }
                Ok(bytes) => {
                    let elapsed = start.elapsed().unwrap();
                    let elapsed = (elapsed.as_secs() as f64) +
                        (elapsed.subsec_millis() as f64)/1000.0;

                    println!("Downloaded {} bytes, {} MB/s", bytes, (bytes as f64)/(elapsed*1024.0*1024.0));
                }
            }
            Ok::<_, Error>(())
        })
        .wait()?;

    Ok(())
}

fn main() {

    //let mut rt = tokio::runtime::Runtime::new().unwrap();

    // should be rt.block_on_all, but this block forever in release builds
    tokio::run(lazy(move || {
   // let _ = rt.block_on(lazy(move || -> Result<(), ()> {
        if let Err(err) = run() {
            eprintln!("ERROR: {}", err);
        }
        println!("DONE1");
        Ok(())
    }));

    println!("DONE2");
}
