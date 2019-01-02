extern crate proxmox_backup;

use proxmox_backup::backup::chunker::*;

fn main() {

    let mut buffer = Vec::new();

    for i in 0..1024*1024 {
        for j in 0..4 {
            let byte = ((i >> (j<<3))&0xff) as u8;
            //println!("BYTE {}", byte);
            buffer.push(byte);
        }
    }
    let mut chunker = Chunker::new(512*1024);

    let count = 100;

    let start = std::time::SystemTime::now();
    
    for _i in 0..count {
        let mut pos = 0;
        while pos < buffer.len() {
            let k = chunker.scan(&buffer[pos..]);
            if k == 0 {
                //println!("LAST {}", pos);
                break;
            } else {
                pos += k;
                //println!("CHUNK {}", pos);
            }
        }
    }

    let elapsed = start.elapsed().unwrap();
    let elapsed = (elapsed.as_secs() as f64) +
        (elapsed.subsec_millis() as f64)/1000.0;
    
    let mbytecount = ((count*buffer.len()) as f64) / (1024.0*1024.0);
    let mbytes_per_sec =  mbytecount/elapsed; 
    println!("SPEED = {} MB/s", mbytes_per_sec); 
}
