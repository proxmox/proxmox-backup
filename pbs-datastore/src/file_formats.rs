use endian_trait::Endian;

// WARNING: PLEASE DO NOT MODIFY THOSE MAGIC VALUES

// openssl::sha::sha256(b"Proxmox Backup Catalog file v1.0")[0..8]
pub const PROXMOX_CATALOG_FILE_MAGIC_1_0: [u8; 8] = [145, 253, 96, 249, 196, 103, 88, 213];

// openssl::sha::sha256(b"Proxmox Backup uncompressed blob v1.0")[0..8]
pub const UNCOMPRESSED_BLOB_MAGIC_1_0: [u8; 8] = [66, 171, 56, 7, 190, 131, 112, 161];

//openssl::sha::sha256(b"Proxmox Backup zstd compressed blob v1.0")[0..8]
pub const COMPRESSED_BLOB_MAGIC_1_0: [u8; 8] = [49, 185, 88, 66, 111, 182, 163, 127];

// openssl::sha::sha256(b"Proxmox Backup encrypted blob v1.0")[0..8]
pub const ENCRYPTED_BLOB_MAGIC_1_0: [u8; 8] = [123, 103, 133, 190, 34, 45, 76, 240];

// openssl::sha::sha256(b"Proxmox Backup zstd compressed encrypted blob v1.0")[0..8]
pub const ENCR_COMPR_BLOB_MAGIC_1_0: [u8; 8] = [230, 89, 27, 191, 11, 191, 216, 11];

// openssl::sha::sha256(b"Proxmox Backup fixed sized chunk index v1.0")[0..8]
pub const FIXED_SIZED_CHUNK_INDEX_1_0: [u8; 8] = [47, 127, 65, 237, 145, 253, 15, 205];

// openssl::sha::sha256(b"Proxmox Backup dynamic sized chunk index v1.0")[0..8]
pub const DYNAMIC_SIZED_CHUNK_INDEX_1_0: [u8; 8] = [28, 145, 78, 165, 25, 186, 179, 205];

/// Data blob binary storage format
///
/// The format start with a 8 byte magic number to identify the type,
/// followed by a 4 byte CRC. This CRC is used on the server side to
/// detect file corruption (computed when upload data), so there is
/// usually no need to compute it on the client side.
///
/// Unencrypted blobs simply contain the CRC, followed by the
/// (compressed) data.
///
/// (MAGIC || CRC32 || Data)
///
/// This format is used for blobs (stored in a BackupDir and accessed directly) and chunks (stored
/// in a chunk store and accessed via a ChunkReader / index file).
#[derive(Endian)]
#[repr(C, packed)]
pub struct DataBlobHeader {
    pub magic: [u8; 8],
    pub crc: [u8; 4],
}

/// Encrypted data blob binary storage format
///
/// The ``DataBlobHeader`` for encrypted blobs additionally contains
/// a 16 byte IV, followed by a 16 byte Authenticated Encyrypten (AE)
/// tag, followed by the encrypted data:
///
/// (MAGIC || CRC32 || IV || TAG || EncryptedData).
#[derive(Endian)]
#[repr(C, packed)]
pub struct EncryptedDataBlobHeader {
    pub head: DataBlobHeader,
    pub iv: [u8; 16],
    pub tag: [u8; 16],
}

/// Header size for different file types
///
/// Panics on unknown magic numbers.
pub fn header_size(magic: &[u8; 8]) -> usize {
    match *magic {
        UNCOMPRESSED_BLOB_MAGIC_1_0 => std::mem::size_of::<DataBlobHeader>(),
        COMPRESSED_BLOB_MAGIC_1_0 => std::mem::size_of::<DataBlobHeader>(),
        ENCRYPTED_BLOB_MAGIC_1_0 => std::mem::size_of::<EncryptedDataBlobHeader>(),
        ENCR_COMPR_BLOB_MAGIC_1_0 => std::mem::size_of::<EncryptedDataBlobHeader>(),
        _ => panic!("unknown blob magic"),
    }
}
