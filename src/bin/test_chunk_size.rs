extern crate proxmox_backup;

// also see https://www.johndcook.com/blog/standard_deviation/

use failure::*;
use std::io::{Read, Write};

use proxmox_backup::backup::*;

struct ChunkWriter {
    chunker: Chunker,
    last_chunk: usize,
    chunk_offset: usize,

    chunk_count: usize,

    M_old: f64,
    M_new: f64,
    S_old: f64,
    S_new: f64,
}

impl ChunkWriter {

    fn new(chunk_size: usize) -> Self {
        ChunkWriter {
            chunker: Chunker::new(chunk_size),
            last_chunk: 0,
            chunk_offset: 0,
            chunk_count: 0,

            M_old: 0.0,
            M_new: 0.0,
            S_old: 0.0,
            S_new: 0.0,
        }
    }

    fn record_stat(&mut self, chunk_size: f64) {

        self.chunk_count += 1;

        if self.chunk_count == 1 {
            self.M_old = chunk_size;
            self.M_new = chunk_size;
            self.S_old = 0.0;
        } else {
            self.M_new = self.M_old + (chunk_size - self.M_old)/(self.chunk_count as f64);
            self.S_new = self.S_old +
                (chunk_size - self.M_old)*(chunk_size - self.M_new);
            // set up for next iteration
            self.M_old = self.M_new;
            self.S_old = self.S_new;
        }

        let variance = if self.chunk_count > 1 {
            self.S_new/((self.chunk_count -1)as f64)
        } else { 0.0 };

        let std_deviation = variance.sqrt();
        let deviation_per = (std_deviation*100.0)/self.M_new;
        println!("COUNT {:10} SIZE {:10} MEAN {:10} DEVIATION {:3}%", self.chunk_count, chunk_size, self.M_new as usize, deviation_per as usize);
    }
}

impl Write for ChunkWriter {

    fn write(&mut self, data: &[u8]) -> std::result::Result<usize, std::io::Error> {

        let chunker = &mut self.chunker;

        let pos = chunker.scan(data);

        if pos > 0 {
            self.chunk_offset += pos;

            let chunk_size = self.chunk_offset - self.last_chunk;

            self.record_stat(chunk_size as f64);

            self.last_chunk = self.chunk_offset;
            Ok(pos)

        } else {
            self.chunk_offset += data.len();
            Ok(data.len())
        }
    }

    fn flush(&mut self) -> std::result::Result<(), std::io::Error> {
        Ok(())
    }
}

fn main() -> Result<(), Error> {

    let mut file = std::fs::File::open("/dev/urandom")?;

    let mut bytes = 0;

    let mut buffer = [0u8; 64*1024];

    let mut writer = ChunkWriter::new(4096*1024);

    loop {

        file.read_exact(&mut buffer)?;
        bytes += buffer.len();

        writer.write_all(&buffer)?;

    }

    Ok(())
}
