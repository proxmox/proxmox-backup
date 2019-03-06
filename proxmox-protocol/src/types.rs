use std::borrow::Borrow;

use endian_trait::Endian;
use failure::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndexType {
    Fixed,
    Dynamic,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct FixedChunk(pub [u8; 32]);

impl FixedChunk {
    pub fn new(hash: [u8; 32]) -> Self {
        Self(hash)
    }

    pub fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, Error> {
        Ok(Self::new(crate::tools::parse_hex_digest(hex.as_ref())?))
    }

    pub fn from_data(data: &[u8]) -> Self {
        let mut hasher = openssl::sha::Sha256::new();
        hasher.update(data);
        Self::new(hasher.finish())
    }

    pub fn digest_to_hex(&self) -> String {
        crate::tools::digest_to_hex(&self.0)
    }
}

impl Endian for FixedChunk {
    fn to_be(self) -> Self {
        self
    }
    fn to_le(self) -> Self {
        self
    }
    fn from_be(self) -> Self {
        self
    }
    fn from_le(self) -> Self {
        self
    }
}

#[derive(Clone, Copy, Debug, Hash)]
#[repr(C, packed)]
pub struct ChunkEntry {
    pub hash: [u8; 32],
    pub size: u64,
}

impl ChunkEntry {
    pub fn new(hash: [u8; 32], size: u64) -> Self {
        Self { hash, size }
    }

    pub fn from_hex<T: AsRef<[u8]>>(hex: T, size: u64) -> Result<Self, Error> {
        Ok(Self::new(
            crate::tools::parse_hex_digest(hex.as_ref())?,
            size,
        ))
    }

    pub fn len(&self) -> u64 {
        self.size
    }

    pub fn from_data(data: &[u8]) -> Self {
        let mut hasher = openssl::sha::Sha256::new();
        hasher.update(data);
        Self::new(hasher.finish(), data.len() as u64)
    }

    pub fn digest_to_hex(&self) -> String {
        crate::tools::digest_to_hex(&self.hash)
    }

    pub fn to_fixed(&self) -> FixedChunk {
        FixedChunk(self.hash)
    }
}

impl PartialEq for ChunkEntry {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size && self.hash == other.hash
    }
}

impl Eq for ChunkEntry {}

impl Endian for ChunkEntry {
    fn to_be(self) -> Self {
        self.size.to_be();
        self
    }

    fn to_le(self) -> Self {
        self.size.to_le();
        self
    }

    fn from_be(self) -> Self {
        self.size.from_be();
        self
    }

    fn from_le(self) -> Self {
        self.size.from_le();
        self
    }
}

impl Into<FixedChunk> for ChunkEntry {
    fn into(self) -> FixedChunk {
        FixedChunk(self.hash)
    }
}

impl Borrow<FixedChunk> for ChunkEntry {
    fn borrow(&self) -> &FixedChunk {
        unsafe { std::mem::transmute(&self.hash) }
    }
}

impl Borrow<FixedChunk> for [u8; 32] {
    fn borrow(&self) -> &FixedChunk {
        unsafe { std::mem::transmute(self) }
    }
}
