use std::sync::Arc;

use failure::*;
use futures::*;
use hyper::{Request, Response, Body};
use openssl::ssl::{SslMethod, SslAcceptor, SslFiletype};
use tokio::net::{TcpListener, TcpStream};

use proxmox_backup::configdir;

// Simple H2 server to test H2 speed with h2s-client.rs

#[tokio::main]
async fn main() -> Result<(), Error> {
    let key_path = configdir!("/proxy.key");
    let cert_path = configdir!("/proxy.pem");

    let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    acceptor.set_private_key_file(key_path, SslFiletype::PEM)
        .map_err(|err| format_err!("unable to read proxy key {} - {}", key_path, err))?;
    acceptor.set_certificate_chain_file(cert_path)
        .map_err(|err| format_err!("unable to read proxy cert {} - {}", cert_path, err))?;
    acceptor.check_private_key().unwrap();

    let acceptor = Arc::new(acceptor.build());

    let listener = TcpListener::bind(std::net::SocketAddr::from(([127,0,0,1], 8008))).await?;

    println!("listening on {:?}", listener.local_addr());

    let mut incoming = listener.incoming();
    while let Some(socket) = incoming.try_next().await? {
        tokio::spawn(handle_connection(socket, Arc::clone(&acceptor))
            .map(|res| {
                if let Err(err) = res {
                    eprintln!("Error: {}", err);
                }
            }));
    }

    Ok(())
}

async fn handle_connection(
    socket: TcpStream,
    acceptor: Arc<SslAcceptor>,
) -> Result<(), Error> {
    socket.set_nodelay(true).unwrap();
    socket.set_send_buffer_size(1024*1024).unwrap();
    socket.set_recv_buffer_size(1024*1024).unwrap();

    let socket = tokio_openssl::accept(acceptor.as_ref(), socket).await?;

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
        future::ok::<_, Error>(response)
    });

    http.serve_connection(socket, service)
        .map_err(Error::from)
        .await?;

    println!("H2 connection CLOSE !");
    Ok(())
}
