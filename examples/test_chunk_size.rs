extern crate proxmox_backup;

// also see https://www.johndcook.com/blog/standard_deviation/

use anyhow::Error;
use std::io::{Read, Write};

use pbs_datastore::Chunker;

struct ChunkWriter {
    chunker: Chunker,
    last_chunk: usize,
    chunk_offset: usize,

    chunk_count: usize,

    m_old: f64,
    m_new: f64,
    s_old: f64,
    s_new: f64,
}

impl ChunkWriter {
    fn new(chunk_size: usize) -> Self {
        ChunkWriter {
            chunker: Chunker::new(chunk_size),
            last_chunk: 0,
            chunk_offset: 0,
            chunk_count: 0,

            m_old: 0.0,
            m_new: 0.0,
            s_old: 0.0,
            s_new: 0.0,
        }
    }

    fn record_stat(&mut self, chunk_size: f64) {
        self.chunk_count += 1;

        if self.chunk_count == 1 {
            self.m_old = chunk_size;
            self.m_new = chunk_size;
            self.s_old = 0.0;
        } else {
            self.m_new = self.m_old + (chunk_size - self.m_old) / (self.chunk_count as f64);
            self.s_new = self.s_old + (chunk_size - self.m_old) * (chunk_size - self.m_new);
            // set up for next iteration
            self.m_old = self.m_new;
            self.s_old = self.s_new;
        }

        let variance = if self.chunk_count > 1 {
            self.s_new / ((self.chunk_count - 1) as f64)
        } else {
            0.0
        };

        let std_deviation = variance.sqrt();
        let deviation_per = (std_deviation * 100.0) / self.m_new;
        println!(
            "COUNT {:10} SIZE {:10} MEAN {:10} DEVIATION {:3}%",
            self.chunk_count, chunk_size, self.m_new as usize, deviation_per as usize
        );
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

    let mut buffer = [0u8; 64 * 1024];

    let mut writer = ChunkWriter::new(4096 * 1024);

    loop {
        file.read_exact(&mut buffer)?;
        bytes += buffer.len();

        writer.write_all(&buffer)?;

        if bytes > 1024 * 1024 * 1024 {
            break;
        }
    }

    Ok(())
}
