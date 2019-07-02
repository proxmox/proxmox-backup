use failure::*;
use futures::*;

// Simple H2 server to test H2 speed with h2s-client.rs

use hyper::{Request, Response, Body};
use tokio::net::TcpListener;

use proxmox_backup::configdir;

use openssl::ssl::{SslMethod, SslAcceptor, SslFiletype};
use std::sync::Arc;
use tokio_openssl::SslAcceptorExt;

pub fn main() -> Result<(), Error> {

    start_h2_server()?;

    Ok(())
}

pub fn start_h2_server() -> Result<(), Error> {

    let key_path = configdir!("/proxy.key");
    let cert_path = configdir!("/proxy.pem");

    let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    acceptor.set_private_key_file(key_path, SslFiletype::PEM)
        .map_err(|err| format_err!("unable to read proxy key {} - {}", key_path, err))?;
    acceptor.set_certificate_chain_file(cert_path)
        .map_err(|err| format_err!("unable to read proxy cert {} - {}", cert_path, err))?;
    acceptor.check_private_key().unwrap();

    let acceptor = Arc::new(acceptor.build());

    let listener = TcpListener::bind(&"127.0.0.1:8008".parse().unwrap()).unwrap();

    println!("listening on {:?}", listener.local_addr());

    let server = listener
        .incoming()
        .map_err(Error::from)
        .and_then(move |sock| {
            sock.set_nodelay(true).unwrap();
            sock.set_send_buffer_size(1024*1024).unwrap();
            sock.set_recv_buffer_size(1024*1024).unwrap();
            acceptor.accept_async(sock).map_err(|e| e.into())
        })
        .then(|r| match r {
            // accept()s can fail here with an Err() when eg. the client rejects
            // the cert and closes the connection, so we follow up with mapping
            // it to an option and then filtering None with filter_map
            Ok(c) => Ok::<_, Error>(Some(c)),
            Err(e) => {
                if let Some(_io) = e.downcast_ref::<std::io::Error>() {
                    // "real" IO errors should not simply be ignored
                    bail!("shutting down...");
                } else {
                    // handshake errors just get filtered by filter_map() below:
                    Ok(None)
                }
            }
        })
        .filter_map(|r| {
            // Filter out the Nones
            r
        })
        .for_each(move |socket| {

            let mut http = hyper::server::conn::Http::new();
            http.http2_only(true);
            // increase window size: todo - find optiomal size
            let max_window_size = (1 << 31) - 2;
            http.http2_initial_stream_window_size(max_window_size);
            http.http2_initial_connection_window_size(max_window_size);

            let service = hyper::service::service_fn(|_req: Request<Body>| {
                println!("Got request");
                let buffer = vec![65u8; 1024*1024]; // nonsense [A,A,A,A...]
                let body = Body::from(buffer);

                let response = Response::builder()
                    .status(http::StatusCode::OK)
                    .header(http::header::CONTENT_TYPE, "application/octet-stream")
                    .body(body)
                    .unwrap();
                Ok::<_, Error>(response)
            });
            http.serve_connection(socket, service)
                .map_err(Error::from)
        })
        .and_then(|_| {
            println!("H2 connection CLOSE !");
            Ok(())
        })
        .then(|res| {
            if let Err(e) = res {
                println!("  -> err={:?}", e);
            }
            Ok(())
        });

    tokio::run(server);

    Ok(())
}
