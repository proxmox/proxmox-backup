use std::collections::HashMap;
use std::ops::Range;

#[derive(Clone)]
pub struct ChunkReadInfo {
    pub range: Range<u64>,
    pub digest: [u8; 32],
}

impl ChunkReadInfo {
    #[inline]
    pub fn size(&self) -> u64 {
        self.range.end - self.range.start
    }
}

/// Trait to get digest list from index files
///
/// To allow easy iteration over all used chunks.
pub trait IndexFile {
    fn index_count(&self) -> usize;
    fn index_digest(&self, pos: usize) -> Option<&[u8; 32]>;
    fn index_bytes(&self) -> u64;
    fn chunk_info(&self, pos: usize) -> Option<ChunkReadInfo>;
    fn index_ctime(&self) -> i64;
    fn index_size(&self) -> usize;

    /// Get the chunk index and the relative offset within it for a byte offset
    fn chunk_from_offset(&self, offset: u64) -> Option<(usize, u64)>;

    /// Compute index checksum and size
    fn compute_csum(&self) -> ([u8; 32], u64);

    /// Returns most often used chunks
    fn find_most_used_chunks(&self, max: usize) -> HashMap<[u8; 32], usize> {
        let mut map = HashMap::new();

        for pos in 0..self.index_count() {
            let digest = self.index_digest(pos).unwrap();

            let count = map.entry(*digest).or_insert(0);
            *count += 1;
        }

        let mut most_used = Vec::new();

        for (digest, count) in map {
            if count <= 1 {
                continue;
            }
            match most_used.binary_search_by_key(&count, |&(_digest, count)| count) {
                Ok(p) => most_used.insert(p, (digest, count)),
                Err(p) => most_used.insert(p, (digest, count)),
            }

            if most_used.len() > max {
                let _ = most_used.pop();
            }
        }

        let mut map = HashMap::new();

        for data in most_used {
            map.insert(data.0, data.1);
        }

        map
    }
}
