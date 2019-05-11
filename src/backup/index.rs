pub trait IndexFile: Send {
    fn index_count(&self) -> usize;
    fn index_digest(&self, pos: usize) -> Option<&[u8; 32]>;
}

/// This struct can read the list of chunks from an `IndexFile`
///
/// The reader simply returns a birary stream of 32 byte digest values.
pub struct ChunkListReader {
    index: Box<dyn IndexFile>,
    pos: usize,
    count: usize,
}

impl ChunkListReader {

    pub fn new(index: Box<dyn IndexFile>) -> Self {
        let count = index.index_count();
        Self { index, pos: 0, count }
    }
}

impl std::io::Read for ChunkListReader {

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        if buf.len() < 32 { panic!("read buffer too small"); }
        if self.pos < self.count {
            let mut written = 0;
            loop {
                let digest = self.index.index_digest(self.pos).unwrap();
                unsafe { std::ptr::copy_nonoverlapping(digest.as_ptr(), buf.as_mut_ptr().add(written), 32); }
                self.pos += 1;
                written += 32;
                if self.pos >= self.count { break; }
                if (written + 32) >= buf.len() { break; }
            }
            return Ok(written);
        } else {
            return Ok(0);
        }
    }
}
