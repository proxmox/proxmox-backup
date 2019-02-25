pub struct ChunkStat {
    pub size: u64,
    pub compressed_size: u64,
    pub disk_size: u64,

    pub chunk_count: usize,
    pub duplicate_chunks: usize,
}

impl ChunkStat {

    pub fn new(size: u64) -> Self {
        ChunkStat {
            size,
            compressed_size: 0,
            disk_size: 0,

            chunk_count: 0,
            duplicate_chunks: 0,
        }
    }
}

impl std::fmt::Debug for ChunkStat {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let avg = ((self.size as f64)/(self.chunk_count as f64)) as usize;
        let compression = (self.compressed_size*100)/(self.size as u64);
        let rate = (self.disk_size*100)/(self.size as u64);
        write!(f, "Size: {}, average chunk size: {}, compression rate: {}%, disk_size: {} ({}%)",
               self.size, avg, compression, self.disk_size, rate)
    }
}
