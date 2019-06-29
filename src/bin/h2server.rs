use failure::*;
use futures::*;

// Simple H2 server to test H2 speed with h2client.rs

use tokio::net::TcpListener;

use proxmox_backup::client::pipe_to_stream::*;

pub fn main() -> Result<(), Error> {

    start_h2_server()?;

    Ok(())
}

pub fn start_h2_server() -> Result<(), Error> {

    let listener = TcpListener::bind(&"127.0.0.1:8008".parse().unwrap()).unwrap();

    println!("listening on {:?}", listener.local_addr());

    let server = listener.incoming().for_each(move |socket| {

        let connection = h2::server::handshake(socket)
            .map_err(Error::from)
            .and_then(|conn| {
                println!("H2 connection bound");

                conn
                    .map_err(Error::from)
                    .for_each(|(request, mut respond)| {
                        println!("GOT request: {:?}", request);

                        let response = http::Response::builder().status(http::StatusCode::OK).body(()).unwrap();

                        let send = respond.send_response(response, false).unwrap();
                        let data = vec![65u8; 1024*1024];
                        PipeToSendStream::new(bytes::Bytes::from(data), send)
                            .and_then(|_| {
                                println!("DATA SENT");
                                Ok(())
                            })
                    })
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

        tokio::spawn(Box::new(connection));
        Ok(())
    })
    .map_err(|e| eprintln!("accept error: {}", e));

    tokio::run(server);

    Ok(())
}
