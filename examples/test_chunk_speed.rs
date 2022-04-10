extern crate proxmox_backup;

use pbs_datastore::Chunker;

fn main() {
    let mut buffer = Vec::new();

    for i in 0..20 * 1024 * 1024 {
        for j in 0..4 {
            let byte = ((i >> (j << 3)) & 0xff) as u8;
            //println!("BYTE {}", byte);
            buffer.push(byte);
        }
    }
    let mut chunker = Chunker::new(64 * 1024);

    let count = 5;

    let start = std::time::SystemTime::now();

    let mut chunk_count = 0;

    for _i in 0..count {
        let mut pos = 0;
        let mut _last = 0;
        while pos < buffer.len() {
            let k = chunker.scan(&buffer[pos..]);
            if k == 0 {
                //println!("LAST {}", pos);
                break;
            } else {
                _last = pos;
                pos += k;
                chunk_count += 1;
                //println!("CHUNK {} {}", pos, pos-last);
            }
        }
    }

    let elapsed = start.elapsed().unwrap();
    let elapsed = (elapsed.as_secs() as f64) + (elapsed.subsec_millis() as f64) / 1000.0;

    let mbytecount = ((count * buffer.len()) as f64) / (1024.0 * 1024.0);
    let avg_chunk_size = mbytecount / (chunk_count as f64);
    let mbytes_per_sec = mbytecount / elapsed;
    println!(
        "SPEED = {} MB/s, avg chunk size = {} KB",
        mbytes_per_sec,
        avg_chunk_size * 1024.0
    );
}
