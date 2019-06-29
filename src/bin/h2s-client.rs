use failure::*;
use futures::*;

// Simple H2 client to test H2 download speed using h2s-server.rs

fn build_client() -> hyper::client::Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>> {
    let mut builder = native_tls::TlsConnector::builder();
    builder.danger_accept_invalid_certs(true);
    let tlsconnector = builder.build().unwrap();
    let mut httpc = hyper::client::HttpConnector::new(1);
    httpc.set_nodelay(true); // important for h2 download performance!
    httpc.enforce_http(false); // we want https...
    let mut https = hyper_tls::HttpsConnector::from((httpc, tlsconnector));
    https.https_only(true); // force it!
    hyper::client::Client::builder()
        .http2_only(true)
        .http2_initial_stream_window_size( (1 << 31) - 2)
        .http2_initial_connection_window_size( (1 << 31) - 2)
         // howto?? .http2_max_frame_size(4*1024*1024) ??
        .build::<_, hyper::Body>(https)
}

pub fn main() -> Result<(), Error> {

    let client = build_client();

    let start = std::time::SystemTime::now();

    let task = futures::stream::repeat(())
        .take(100)
        .and_then(move |_| {
            let request = http::Request::builder()
                .method("GET")
                .uri("https://localhost:8008/")
                .body(hyper::Body::empty())
                .unwrap();

            client
                .request(request)
                .map_err(Error::from)
                .and_then(|resp| {
                    resp.into_body()
                        .map_err(Error::from)
                        .fold(0, move |mut acc, chunk| {
                            println!("got frame {}", chunk.len());
                            acc += chunk.len();
                            Ok::<_, Error>(acc)
                        })
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
            Ok(())
        });

    tokio::run(task);

    Ok(())
}
