use failure::*;
use futures::*;

// Simple H2 client to test H2 download speed using h2server.rs

use tokio::net::TcpStream;

struct Process {
    body: h2::RecvStream,
    trailers: bool,
    bytes: usize,
}

impl Future for Process {
    type Item = usize;
    type Error = Error;

    fn poll(&mut self) -> Poll<usize, Error> {
        loop {
            if self.trailers {
                let trailers = try_ready!(self.body.poll_trailers());
                if let Some(trailers) = trailers {
                    println!("trailers: {:?}", trailers);
                }
                println!("Received {} bytes", self.bytes);

                return Ok(Async::Ready(self.bytes));
            } else {
                match try_ready!(self.body.poll()) {
                    Some(chunk) => {
                        self.body.release_capacity().release_capacity(chunk.len())?;
                        self.bytes += chunk.len();
                        // println!("GOT FRAME {}", chunk.len());
                    },
                    None => {
                        self.trailers = true;
                    },
                }
            }
        }
    }
}

fn send_request(mut client: h2::client::SendRequest<bytes::Bytes>) -> impl Future<Item=usize, Error=Error> {

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

pub fn main() -> Result<(), Error> {

    let tcp_stream = TcpStream::connect(&"127.0.0.1:8008".parse().unwrap());

    let start = std::time::SystemTime::now();

    let tcp = tcp_stream
        .map_err(Error::from)
        .and_then(|c| {
            h2::client::Builder::new()
                .initial_connection_window_size(1024*1024*1024)
                .initial_window_size(1024*1024*1024)
                .max_frame_size(4*1024*1024)
                .handshake(c)
                .map_err(Error::from)
        })
        .and_then(|(client, h2)| {

            // Spawn a task to run the conn...
            tokio::spawn(h2.map_err(|e| println!("GOT ERR={:?}", e)));

            futures::stream::repeat(())
                .take(2000)
                .and_then(move |_| send_request(client.clone()))
                .fold(0, move |mut acc, size| {
                    acc += size;
                    Ok::<_, Error>(acc)
                })
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
            Ok(())
        });

    tokio::run(tcp);

    Ok(())
}
