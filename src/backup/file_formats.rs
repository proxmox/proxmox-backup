use endian_trait::Endian;

// WARNING: PLEASE DO NOT MODIFY THOSE MAGIC VALUES

// openssl::sha::sha256(b"Proxmox Backup uncompressed chunk v1.0")[0..8]
pub const UNCOMPRESSED_CHUNK_MAGIC_1_0: [u8; 8] = [79, 127, 200, 4, 121, 74, 135, 239];

// openssl::sha::sha256(b"Proxmox Backup encrypted chunk v1.0")[0..8]
pub const ENCRYPTED_CHUNK_MAGIC_1_0: [u8; 8] = [8, 54, 114, 153, 70, 156, 26, 151];

// openssl::sha::sha256(b"Proxmox Backup zstd compressed chunk v1.0")[0..8]
pub const COMPRESSED_CHUNK_MAGIC_1_0: [u8; 8] = [191, 237, 46, 195, 108, 17, 228, 235];

// openssl::sha::sha256(b"Proxmox Backup zstd compressed encrypted chunk v1.0")[0..8]
pub const ENCR_COMPR_CHUNK_MAGIC_1_0: [u8; 8] = [9, 40, 53, 200, 37, 150, 90, 196];

// openssl::sha::sha256(b"Proxmox Backup uncompressed blob v1.0")[0..8]
pub const UNCOMPRESSED_BLOB_MAGIC_1_0: [u8; 8] = [66, 171, 56, 7, 190, 131, 112, 161];

//openssl::sha::sha256(b"Proxmox Backup zstd compressed blob v1.0")[0..8]
pub const COMPRESSED_BLOB_MAGIC_1_0: [u8; 8] = [49, 185, 88, 66, 111, 182, 163, 127];

// openssl::sha::sha256(b"Proxmox Backup encrypted blob v1.0")[0..8]
pub const ENCRYPTED_BLOB_MAGIC_1_0: [u8; 8] = [123, 103, 133, 190, 34, 45, 76, 240];

// openssl::sha::sha256(b"Proxmox Backup zstd compressed encrypted blob v1.0")[0..8]
pub const ENCR_COMPR_BLOB_MAGIC_1_0: [u8; 8] = [230, 89, 27, 191, 11, 191, 216, 11];

//openssl::sha::sha256(b"Proxmox Backup authenticated blob v1.0")[0..8]
pub const AUTHENTICATED_BLOB_MAGIC_1_0: [u8; 8] = [31, 135, 238, 226, 145, 206, 5, 2];

//openssl::sha::sha256(b"Proxmox Backup zstd compressed authenticated blob v1.0")[0..8]
pub const AUTH_COMPR_BLOB_MAGIC_1_0: [u8; 8] = [126, 166, 15, 190, 145, 31, 169, 96];

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
/// This is basically the same format we use for chunks, but
/// with other magic numbers so that we can distinguish them.
#[derive(Endian)]
#[repr(C,packed)]
pub struct DataBlobHeader {
    pub magic: [u8; 8],
    pub crc: [u8; 4],
}

/// Authenticated data blob binary storage format
///
/// The ``DataBlobHeader`` for authenticated blobs additionally contains
/// a 16 byte HMAC tag, followed by the data:
///
/// (MAGIC || CRC32 || TAG || Data).
#[derive(Endian)]
#[repr(C,packed)]
pub struct AuthenticatedDataBlobHeader {
    pub head: DataBlobHeader,
    pub tag: [u8; 32],
}

/// Encrypted data blob binary storage format
///
/// The ``DataBlobHeader`` for encrypted blobs additionally contains
/// a 16 byte IV, followed by a 16 byte Authenticated Encyrypten (AE)
/// tag, followed by the encrypted data:
///
/// (MAGIC || CRC32 || IV || TAG || EncryptedData).
#[derive(Endian)]
#[repr(C,packed)]
pub struct EncryptedDataBlobHeader {
    pub head: DataBlobHeader,
    pub iv: [u8; 16],
    pub tag: [u8; 16],
}

/// Data chunk binary storage format
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
#[derive(Endian)]
#[repr(C,packed)]
pub struct DataChunkHeader {
    pub magic: [u8; 8],
    pub crc: [u8; 4],
}

/// Encrypted Data chunk binary storage format
///
/// The ``DataChunkHeader`` for encrypted chunks additionally contains
/// a 16 byte IV, followed by a 16 byte Authenticated Encyrypten (AE)
/// tag, followed by the encrypted data:
///
/// (MAGIC || CRC32 || IV || TAG || EncryptedData).
#[derive(Endian)]
#[repr(C,packed)]
pub struct EncryptedDataChunkHeader {
    pub head: DataChunkHeader,
    pub iv: [u8; 16],
    pub tag: [u8; 16],
}
