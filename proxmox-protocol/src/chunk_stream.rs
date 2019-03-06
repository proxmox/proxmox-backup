use std::io::Read;

use failure::Error;

use crate::Chunker;

pub struct ChunkStream<T: Read> {
    input: T,
    buffer: Vec<u8>,
    fill: usize,
    pos: usize,
    keep: bool,
    eof: bool,
    chunker: Chunker,
}

impl<T: Read> ChunkStream<T> {
    pub fn new(input: T) -> Self {
        Self {
            input,
            buffer: Vec::new(),
            fill: 0,
            pos: 0,
            keep: false,
            eof: false,
            chunker: Chunker::new(4 * 1024 * 1024),
        }
    }

    pub fn stream(&mut self) -> &mut Self {
        self
    }

    fn fill_buf(&mut self) -> Result<bool, Error> {
        if self.fill == self.buffer.len() {
            let mut more = self.buffer.len(); // just double it
            if more == 0 {
                more = 1024 * 1024; // at the start, make a 1M buffer
            }
            // we need more data:
            self.buffer.reserve(more);
            unsafe {
                self.buffer.set_len(self.buffer.capacity());
            }
        }

        match self.input.read(&mut self.buffer[self.fill..]) {
            Ok(more) => {
                if more == 0 {
                    self.eof = true;
                }
                self.fill += more;
                Ok(true)
            }
            Err(err) => {
                if err.kind() == std::io::ErrorKind::WouldBlock {
                    Ok(false)
                } else {
                    Err(err.into())
                }
            }
        }
    }

    fn consume(&mut self) {
        assert!(self.fill >= self.pos);

        let remaining = self.fill - self.pos;
        unsafe {
            std::ptr::copy_nonoverlapping(
                &self.buffer[self.pos] as *const u8,
                self.buffer.as_mut_ptr(),
                remaining,
            );
        }
        self.fill = remaining;
        self.pos = 0;
    }

    pub fn next(&mut self) {
        self.keep = false;
    }

    // This crate should not depend on the futures create, so we use another Option instead of
    // Async<T>.
    pub fn get(&mut self) -> Result<Option<Option<&[u8]>>, Error> {
        if self.keep {
            return Ok(Some(Some(&self.buffer[0..self.pos])));
        }

        if self.eof {
            return Ok(Some(None));
        }

        if self.pos != 0 {
            self.consume();
        }

        loop {
            match self.fill_buf() {
                Ok(true) => (),
                Ok(false) => return Ok(None),
                Err(err) => return Err(err),
            }

            // Note that if we hit EOF we hit a hard boundary...
            let boundary = self.chunker.scan(&self.buffer[self.pos..self.fill]);
            if boundary == 0 && !self.eof {
                self.pos = self.fill;
                continue;
            }

            self.pos += boundary;
            self.keep = true;
            return Ok(Some(Some(&self.buffer[0..self.pos])));
        }
    }
}
