use std::mem;

use endian_trait::Endian;

// There's no reason to have more than that in a single packet...
pub const MAX_PACKET_SIZE: u32 = 16 * 1024 * 1024;

// Each packet has a transaction ID (eg. when uploading multiple disks each
// upload is a separate stream).
#[derive(Endian)]
#[repr(C, packed)]
pub struct Packet {
    pub id: u8,      // request/command id
    pub pkttype: u8, // packet type
    pub length: u32, // data length before the next packet

                     // content is attached directly afterwards
}

impl Packet {
    pub fn builder(id: u8, pkttype: PacketType) -> PacketBuilder {
        PacketBuilder::new(id, pkttype)
    }

    pub fn simple(id: u8, pkttype: PacketType) -> Vec<u8> {
        Self::builder(id, pkttype).finish()
    }
}

#[derive(Endian, Clone, Copy)]
#[repr(u8)]
pub enum ErrorId {
    Generic,
    Busy,
}

#[repr(u8)]
#[derive(Clone, Copy)]
pub enum PacketType {
    /// First packet sent by the server.
    Hello,

    /// Generic acknowledgement.
    Ok,

    /// Error packet sent by the server, this cancels the request for which it is produced.
    Error,

    /// The client wants the list of available hashes in order to know which ones require an
    /// upload.
    ///
    /// The body should contain a backup file name of which to retrieve the hash list.
    ///
    /// Server responds with a sequence of ``HashListPart`` packets.
    GetHashList,

    /// Array of hashes. The number of hashes in a packet is calculated from the packet length as
    /// provided in the ``Packet`` struct. An empty packet indicates the end of the list. We send a
    /// sequence of such packets because we don't know whether the server will be keeping the list
    /// in memory yet, so it might not know the number in advance and may be iterating through
    /// directories until it hits an end. It can produce the network packets asynchronously while
    /// walking the chunk dir.
    HashListPart,

    /// Client requests to download chunks via a hash list from the server. The number of chunks
    /// can be derived from the length of this request, so it works similar to ``HashListPart``,
    /// but there's only 1 chunk list per request ID.
    ///
    /// The server responds with a sequence of ``Chunk`` packets or ``Error``.
    DownloadChunks,

    /// The response to ``DownloadChunks``. One packet per requested chunk.
    Chunk,

    /// The upload of a chunk can happen independent from the ongoing backup
    /// streams. Server responds with an ``OK``.
    UploadChunk,

    /// Create a file in a new or existing backup. Contains all the metadata of
    /// a file.
    ///
    /// The server responds with ``BackupCreated`` or ``Error``. On ``BackupCreated`` the client
    /// may proceed to send as many ``BackupData...`` packets as necessary to fill the file.
    /// The sequence is finished by the client with a ``BackupFinished``.
    CreateBackup,

    /// Successful from the server to a client's ``CreateBackup`` packet. Contains the server side
    /// path relative to the store.
    BackupCreated,

    /// This packet contains an array of references to fixed sized chunks.  Clients should upload
    /// chunks via ``UploadChunk`` packets before using them in this type of packet. A non-existent
    /// chunk is an error.
    ///
    /// The server produces an ``Error`` packet in case of an error.
    BackupDataFixed,

    /// This packet contains an array of references to dynamic sized chunks.  Clients should upload
    /// chunks via ``UploadChunk`` packets before using them in this type of packet. A non-existent
    /// chunk is an error.
    ///
    /// The server produces an ``Error`` packet in case of an error.
    BackupDataDynamic,

    /// This ends a backup file. The server responds with an ``OK`` or an ``Error`` packet.
    BackupFinished,
}

// Nightly has a std::convert::TryFrom, actually...
impl PacketType {
    pub fn try_from(v: u8) -> Option<Self> {
        if v <= PacketType::BackupFinished as u8 {
            Some(unsafe { std::mem::transmute(v) })
        } else {
            None
        }
    }
}

// Not using bitflags! for Endian derive...
pub mod backup_flags {
    pub type Type = u8;
    /// The backup must not exist yet.
    pub const EXCL: Type = 0x00000001;
    /// The data represents a raw file
    pub const RAW: Type = 0x00000002;
    /// The data uses dynamically sized chunks (catar file)
    pub const DYNAMIC_CHUNKS: Type = 0x00000004;
}

pub mod backup_type {
    pub type Type = u8;
    pub const VM: Type = 0;
    pub const CT: Type = 1;
    pub const HOST: Type = 2;

    use failure::{bail, Error};
    pub fn id_to_name(id: Type) -> Result<&'static str, Error> {
        Ok(match id {
            VM => "vm",
            CT => "ct",
            HOST => "host",
            n => bail!("unknown backup type id: {}", n),
        })
    }

    pub fn name_to_id(id: &str) -> Result<Type, Error> {
        Ok(match id {
            "vm" => VM,
            "ct" => CT,
            "host" => HOST,
            n => bail!("unknown backup type name: {}", n),
        })
    }
}

#[repr(C, packed)]
#[derive(Endian)]
pub struct DynamicChunk {
    pub offset: u64,
    pub digest: [u8; 32],
}

pub mod server {
    use endian_trait::Endian;

    pub const PROTOCOL_VERSION: u32 = 1;

    pub const HELLO_MAGIC: [u8; 8] = *b"PMXBCKUP";

    pub const HELLO_VERSION: u32 = 1; // the current version
    #[derive(Endian)]
    #[repr(C, packed)]
    pub struct Hello {
        pub magic: [u8; 8],
        pub version: u32,
    }

    #[derive(Endian)]
    #[repr(C, packed)]
    pub struct Error {
        pub id: u8,
    }

    #[derive(Endian)]
    #[repr(C, packed)]
    pub struct Chunk {
        pub hash: super::DynamicChunk,
        // Data follows here...
    }

    #[derive(Endian)]
    #[repr(C, packed)]
    pub struct BackupCreated {
        pub path_length: u16,
        // path follows here
    }
}

pub mod client {
    use endian_trait::Endian;

    #[derive(Endian)]
    #[repr(C, packed)]
    pub struct UploadChunk {
        pub hash: crate::FixedChunk,
    }

    #[derive(Endian)]
    #[repr(C, packed)]
    pub struct CreateBackup {
        pub backup_type: super::backup_type::Type,
        pub id_length: u8,  // length of the ID string
        pub timestamp: u64, // seconds since the epoch
        pub flags: super::backup_flags::Type,
        pub name_length: u8, // file name length
        pub chunk_size: u32, // average or "fixed" chunk size
        pub file_size: u64,  // size for fixed size files (must be 0 if DYNAMIC_CHUNKS is set)

                             // ``id_length`` bytes of ID follow
                             // ``name_length`` bytes of file name follow
                             // Further packets contain the data or chunks
    }

    #[derive(Endian)]
    #[repr(C, packed)]
    pub struct GetHashList {
        pub name_length: u16,
        // name follows as payload
    }
}

pub struct PacketBuilder {
    data: Vec<u8>,
}

impl PacketBuilder {
    pub fn new(id: u8, pkttype: PacketType) -> Self {
        let data = Vec::with_capacity(mem::size_of::<Packet>());
        let mut me = Self { data };
        me.write_data(
            Packet {
                id,
                pkttype: pkttype as _,
                length: 0,
            }
            .to_le(),
        );
        me
    }

    pub fn reserve(&mut self, more: usize) -> &mut Self {
        self.data.reserve(more);
        self
    }

    pub fn write_buf(&mut self, buf: &[u8]) -> &mut Self {
        self.data.extend_from_slice(buf);
        self
    }

    pub fn write_data<T: Endian>(&mut self, data: T) -> &mut Self {
        self.write_data_noswap(&data.to_le())
    }

    pub fn write_data_noswap<T>(&mut self, data: &T) -> &mut Self {
        self.write_buf(unsafe {
            std::slice::from_raw_parts(data as *const T as *const u8, mem::size_of::<T>())
        })
    }

    pub fn finish(mut self) -> Vec<u8> {
        let length = self.data.len();
        assert!(length >= mem::size_of::<Packet>());
        unsafe {
            let head = self.data.as_mut_ptr() as *mut Packet;
            std::ptr::write_unaligned((&mut (*head).length) as *mut u32, (length as u32).to_le());
        }
        self.data
    }
}
