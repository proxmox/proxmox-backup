use failure::*;
use futures::*;
use std::sync::atomic::{AtomicUsize, Ordering};

extern crate proxmox_backup;

use proxmox_backup::backup::*;

// Test Chunker with real data read from a file.
//
// To generate some test input use:
// # dd if=/dev/urandom of=random-test.dat bs=1M count=1024 iflag=fullblock
//
// Note: I can currently get about 830MB/s

fn main() {

    let repeat = std::sync::Arc::new(AtomicUsize::new(0));
    let repeat2 = repeat.clone();

    let stream_len = std::sync::Arc::new(AtomicUsize::new(0));
    let stream_len2 = stream_len.clone();

    let task = tokio::fs::File::open("random-test.dat")
        .map_err(Error::from)
        .and_then(move |file| {
            let stream = tokio::codec::FramedRead::new(file, tokio::codec::BytesCodec::new())
                .map(|bytes| bytes.to_vec()).map_err(Error::from);
            //let chunk_stream = FixedChunkStream::new(stream, 4*1024*1024);
            let chunk_stream = ChunkStream::new(stream);

            let start_time = std::time::Instant::now();

            chunk_stream
                .for_each(move |chunk| {
                    if chunk.len() > 16*1024*1024 { panic!("Chunk too large {}", chunk.len()); }
                    repeat.fetch_add(1, Ordering::SeqCst);
                    stream_len.fetch_add(chunk.len(), Ordering::SeqCst);
                    println!("Got chunk {}", chunk.len());
                    Ok(())
                })
                .and_then(move |_result| {
                    let repeat = repeat2.load(Ordering::SeqCst);
                    let stream_len = stream_len2.load(Ordering::SeqCst);
                    let speed = ((stream_len*1000000)/(1024*1024))/(start_time.elapsed().as_micros() as usize);
                    println!("Uploaded {} chunks in {} seconds ({} MB/s).", repeat, start_time.elapsed().as_secs(), speed);
                    println!("Average chunk size was {} bytes.", stream_len/repeat);
                    println!("time per request: {} microseconds.", (start_time.elapsed().as_micros())/(repeat as u128));
                    Ok(())
                })
        });

    tokio::run(task.map_err(|err| { panic!("ERROR: {}", err); }));
}
