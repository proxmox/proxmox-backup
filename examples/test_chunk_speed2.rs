use anyhow::Error;
use futures::*;

extern crate proxmox_backup;

use pbs_client::ChunkStream;

// Test Chunker with real data read from a file.
//
// To generate some test input use:
// # dd if=/dev/urandom of=random-test.dat bs=1M count=1024 iflag=fullblock
//
// Note: I can currently get about 830MB/s

fn main() {
    if let Err(err) = proxmox_async::runtime::main(run()) {
        panic!("ERROR: {}", err);
    }
}

async fn run() -> Result<(), Error> {
    let file = tokio::fs::File::open("random-test.dat").await?;

    let stream = tokio_util::codec::FramedRead::new(file, tokio_util::codec::BytesCodec::new())
        .map_ok(|bytes| bytes.to_vec())
        .map_err(Error::from);

    //let chunk_stream = FixedChunkStream::new(stream, 4*1024*1024);
    let mut chunk_stream = ChunkStream::new(stream, None);

    let start_time = std::time::Instant::now();

    let mut repeat = 0;
    let mut stream_len = 0;
    while let Some(chunk) = chunk_stream.try_next().await? {
        if chunk.len() > 16 * 1024 * 1024 {
            panic!("Chunk too large {}", chunk.len());
        }

        repeat += 1;
        stream_len += chunk.len();

        println!("Got chunk {}", chunk.len());
    }

    let speed =
        ((stream_len * 1_000_000) / (1024 * 1024)) / (start_time.elapsed().as_micros() as usize);
    println!(
        "Uploaded {} chunks in {} seconds ({} MB/s).",
        repeat,
        start_time.elapsed().as_secs(),
        speed
    );
    println!("Average chunk size was {} bytes.", stream_len / repeat);
    println!(
        "time per request: {} microseconds.",
        (start_time.elapsed().as_micros()) / (repeat as u128)
    );

    Ok(())
}
