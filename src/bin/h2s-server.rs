use failure::*;
use futures::*;
use std::path::Path;

// Simple H2 server to test H2 speed with h2s-client.rs

use hyper::{Request, Response, Body};
use tokio::net::TcpListener;

use proxmox_backup::client::pipe_to_stream::*;
use proxmox_backup::tools;
use proxmox_backup::configdir;

pub fn main() -> Result<(), Error> {

    start_h2_server()?;

    Ok(())
}

fn load_certificate<T: AsRef<Path>, U: AsRef<Path>>(
    key: T,
    cert: U,
) -> Result<openssl::pkcs12::Pkcs12, Error> {
    let key = tools::file_get_contents(key)?;
    let cert = tools::file_get_contents(cert)?;

    let key = openssl::pkey::PKey::private_key_from_pem(&key)?;
    let cert = openssl::x509::X509::from_pem(&cert)?;

    Ok(openssl::pkcs12::Pkcs12::builder()
        .build("", "", &key, &cert)?)
}

pub fn start_h2_server() -> Result<(), Error> {

    let cert_path = configdir!("/proxy.pfx");
    let raw_cert = match std::fs::read(cert_path) {
        Ok(pfx) => pfx,
        Err(ref err) if err.kind() == std::io::ErrorKind::NotFound => {
            let pkcs12 = load_certificate(configdir!("/proxy.key"), configdir!("/proxy.pem"))?;
            pkcs12.to_der()?
        }
        Err(err) => bail!("unable to read certificate file {} - {}", cert_path, err),
    };

    let identity = match native_tls::Identity::from_pkcs12(&raw_cert, "") {
        Ok(data) => data,
        Err(err) => bail!("unable to decode pkcs12 identity {} - {}", cert_path, err),
    };

    let acceptor = native_tls::TlsAcceptor::new(identity)?;
    let acceptor = std::sync::Arc::new(tokio_tls::TlsAcceptor::from(acceptor));

    let listener = TcpListener::bind(&"127.0.0.1:8008".parse().unwrap()).unwrap();

    println!("listening on {:?}", listener.local_addr());

    let server = listener
        .incoming()
        .map_err(Error::from)
        .and_then(move |sock| acceptor.accept(sock).map_err(|e| e.into()))
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

            let service = hyper::service::service_fn(|req: Request<Body>| {
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
