pub struct ChunkStat {
    pub size: u64,
    pub compressed_size: u64,
    pub disk_size: u64,

    pub chunk_count: usize,
    pub duplicate_chunks: usize,

    start_time: std::time::SystemTime,
}

impl ChunkStat {
    pub fn new(size: u64) -> Self {
        ChunkStat {
            size,
            compressed_size: 0,
            disk_size: 0,

            chunk_count: 0,
            duplicate_chunks: 0,

            start_time: std::time::SystemTime::now(),
        }
    }
}

impl std::fmt::Debug for ChunkStat {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let avg = ((self.size as f64) / (self.chunk_count as f64)) as usize;
        let compression = (self.compressed_size * 100) / self.size;
        let rate = (self.disk_size * 100) / self.size;

        let elapsed = self.start_time.elapsed().unwrap();
        let elapsed = (elapsed.as_secs() as f64) + (elapsed.subsec_millis() as f64) / 1000.0;

        let write_speed = ((self.size as f64) / (1024.0 * 1024.0)) / elapsed;

        write!(f, "Size: {}, average chunk size: {}, compression rate: {}%, disk_size: {} ({}%), speed: {:.2} MB/s",
               self.size, avg, compression, self.disk_size, rate, write_speed)
    }
}
