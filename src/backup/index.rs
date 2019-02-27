pub trait IndexFile {
    fn index_count(&self) -> usize;
    fn index_digest(&self, pos: usize) -> Option<&[u8; 32]>;
}

pub struct IndexIterator {
    pos: usize,
    count: usize,
    reader: Box<dyn IndexFile + Send>,
}

impl Iterator for IndexIterator {
    type Item = [u8; 32];

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos == self.count {
            return None;
        }

        let digest = self.reader.index_digest(self.pos).unwrap();
        self.pos += 1;
        Some(*digest)
    }
}

impl From<Box<dyn IndexFile + Send>> for IndexIterator {
    fn from(file: Box<dyn IndexFile + Send>) -> Self {
        Self {
            pos: 0,
            count: file.index_count(),
            reader: file,
        }
    }
}

impl<T: IndexFile + Send + 'static> From<Box<T>> for IndexIterator {
    fn from(file: Box<T>) -> Self {
        Self {
            pos: 0,
            count: file.index_count(),
            reader: file,
        }
    }
}
