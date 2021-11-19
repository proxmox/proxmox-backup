use anyhow::Error;
use futures::*;
use hyper::{Body, Request, Response};

use tokio::net::{TcpListener, TcpStream};

fn main() -> Result<(), Error> {
    proxmox_async::runtime::main(run())
}

async fn run() -> Result<(), Error> {
    let listener = TcpListener::bind(std::net::SocketAddr::from(([127, 0, 0, 1], 8008))).await?;

    println!("listening on {:?}", listener.local_addr());

    loop {
        let (socket, _addr) = listener.accept().await?;
        tokio::spawn(handle_connection(socket).map(|res| {
            if let Err(err) = res {
                eprintln!("Error: {}", err);
            }
        }));
    }
}

async fn handle_connection(socket: TcpStream) -> Result<(), Error> {
    socket.set_nodelay(true).unwrap();

    let mut http = hyper::server::conn::Http::new();
    http.http2_only(true);
    // increase window size: todo - find optiomal size
    let max_window_size = (1 << 31) - 2;
    http.http2_initial_stream_window_size(max_window_size);
    http.http2_initial_connection_window_size(max_window_size);

    let service = hyper::service::service_fn(|_req: Request<Body>| {
        println!("Got request");
        let buffer = vec![65u8; 4 * 1024 * 1024]; // nonsense [A,A,A,A...]
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
