use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::Error;
use futures::future::TryFutureExt;
use futures::stream::Stream;
use tokio::net::TcpStream;

// Simple H2 client to test H2 download speed using h2server.rs

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
                        this.body.flow_control().release_capacity(chunk.len())?;
                        this.bytes += chunk.len();
                        // println!("GOT FRAME {}", chunk.len());
                    }
                    Some(Err(err)) => return Poll::Ready(Err(Error::from(err))),
                    None => {
                        this.trailers = true;
                    }
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

    response.map_err(Error::from).and_then(|response| Process {
        body: response.into_body(),
        trailers: false,
        bytes: 0,
    })
}

fn main() -> Result<(), Error> {
    proxmox_async::runtime::main(run())
}

async fn run() -> Result<(), Error> {
    let start = std::time::SystemTime::now();

    let conn = TcpStream::connect(std::net::SocketAddr::from(([127, 0, 0, 1], 8008))).await?;
    conn.set_nodelay(true).unwrap();

    let (client, h2) = h2::client::Builder::new()
        .initial_connection_window_size(1024 * 1024 * 1024)
        .initial_window_size(1024 * 1024 * 1024)
        .max_frame_size(4 * 1024 * 1024)
        .handshake(conn)
        .await?;

    tokio::spawn(async move {
        if let Err(err) = h2.await {
            println!("GOT ERR={:?}", err);
        }
    });

    let mut bytes = 0;
    for _ in 0..2000 {
        bytes += send_request(client.clone()).await?;
    }

    let elapsed = start.elapsed().unwrap();
    let elapsed = (elapsed.as_secs() as f64) + (elapsed.subsec_millis() as f64) / 1000.0;

    println!(
        "Downloaded {} bytes, {} MB/s",
        bytes,
        (bytes as f64) / (elapsed * 1024.0 * 1024.0)
    );

    Ok(())
}
