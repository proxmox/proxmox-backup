use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use failure::*;
use futures::future::TryFutureExt;
use futures::stream::Stream;

// Simple H2 client to test H2 download speed using h2s-server.rs

struct Process {
    body: h2::RecvStream,
    trailers: bool,
    bytes: usize,
}

impl Future for Process {
    type Output = Result<usize, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            if this.trailers {
                match futures::ready!(this.body.poll_trailers(cx)) {
                    Ok(Some(trailers)) => println!("trailers: {:?}", trailers),
                    Ok(None) => (),
                    Err(err) => return Poll::Ready(Err(Error::from(err))),
                }

                println!("Received {} bytes", this.bytes);

                return Poll::Ready(Ok(this.bytes));
            } else {
                match futures::ready!(Pin::new(&mut this.body).poll_next(cx)) {
                    Some(Ok(chunk)) => {
                        this.body.release_capacity().release_capacity(chunk.len())?;
                        this.bytes += chunk.len();
                        // println!("GOT FRAME {}", chunk.len());
                    },
                    Some(Err(err)) => return Poll::Ready(Err(Error::from(err))),
                    None => {
                        this.trailers = true;
                    },
                }
            }
        }
    }
}

fn send_request(
    mut client: h2::client::SendRequest<bytes::Bytes>,
) -> impl Future<Output = Result<usize, Error>> {
    println!("sending request");

    let request = http::Request::builder()
        .uri("http://localhost/")
        .body(())
        .unwrap();

    let (response, _stream) = client.send_request(request, true).unwrap();

    response
        .map_err(Error::from)
        .and_then(|response| {
            Process { body: response.into_body(), trailers: false, bytes: 0 }
        })
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let start = std::time::SystemTime::now();

    let conn = tokio::net::TcpStream::connect(&"127.0.0.1:8008".parse().unwrap()).await?;

    conn.set_nodelay(true).unwrap();
    conn.set_recv_buffer_size(1024*1024).unwrap();

    use openssl::ssl::{SslConnector, SslMethod};

    let mut ssl_connector_builder = SslConnector::builder(SslMethod::tls()).unwrap();
    ssl_connector_builder.set_verify(openssl::ssl::SslVerifyMode::NONE);
    let conn =
        tokio_openssl::connect(
            ssl_connector_builder.build().configure()?,
            "localhost",
            conn,
        )
        .await
        .map_err(|err| format_err!("connect failed - {}", err))?;

    let (client, h2) = h2::client::Builder::new()
        .initial_connection_window_size(1024*1024*1024)
        .initial_window_size(1024*1024*1024)
        .max_frame_size(4*1024*1024)
        .handshake(conn)
        .await?;

    // Spawn a task to run the conn...
    tokio::spawn(async move {
        if let Err(e) = h2.await {
            println!("GOT ERR={:?}", e);
        }
    });

    let mut bytes = 0;
    for _ in 0..100 {
        match send_request(client.clone()).await {
            Ok(b) => {
                bytes += b;
            }
            Err(e) => {
                println!("ERROR {}", e);
                return Ok(());
            }
        }
    }

    let elapsed = start.elapsed().unwrap();
    let elapsed = (elapsed.as_secs() as f64) +
        (elapsed.subsec_millis() as f64)/1000.0;

    println!("Downloaded {} bytes, {} MB/s", bytes, (bytes as f64)/(elapsed*1024.0*1024.0));

    Ok(())
}
