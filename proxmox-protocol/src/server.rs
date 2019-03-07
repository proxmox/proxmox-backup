use std::collections::hash_map::{self, HashMap};
use std::io::{Read, Write};
use std::{mem, ptr};

use failure::*;

use endian_trait::Endian;

use crate::common;
use crate::protocol::*;
use crate::ChunkEntry;
use crate::FixedChunk;

type Result<T> = std::result::Result<T, Error>;

pub trait ChunkList: Send {
    fn next(&mut self) -> Result<Option<&[u8; 32]>>;
}

/// This provides callbacks used by a `Connection` when it receives a packet.
pub trait HandleClient {
    /// The protocol handler will call this when the client produces an irrecoverable error.
    fn error(&self);

    /// The client wants the list of hashes, the provider should provide an iterator over chunk
    /// entries.
    fn get_chunk_list(&self, backup_name: &str) -> Result<Box<dyn ChunkList>>;

    /// The client has uploaded a chunk, we should add it to the chunk store. Return whether the
    /// chunk was new.
    fn upload_chunk(&self, chunk: &ChunkEntry, data: &[u8]) -> Result<bool>;

    /// The client wants to create a backup. Since multiple backup streams can happen in parallel,
    /// this should return a handler used to create the individual streams.
    /// The handler will be informed about success via the ``finish()`` method.
    fn create_backup(
        &self,
        backup_type: &str,
        id: &str,
        timestamp: i64,
        new: bool,
    ) -> Result<Box<dyn HandleBackup + Send>>;
}

/// A single backup may contain multiple files. Currently we represent this via a hierarchy where
/// the `HandleBackup` trait is instantiated for each backup, which is responsible for
/// instantiating the `BackupFile` trait objects.
pub trait HandleBackup {
    /// All individual streams for this backup have been successfully finished.
    fn finish(&mut self) -> Result<()>;

    /// Create a specific file in this backup.
    fn create_file(
        &self,
        name: &str,
        fixed_size: Option<u64>,
        chunk_size: usize,
    ) -> Result<Box<dyn BackupFile + Send>>;
}

/// This handles backup files created by calling `create_file` on a `Backup`.
pub trait BackupFile {
    /// Backup use the server-local timestamp formatting, so we want to be able to tell the client
    /// the real remote path:
    fn relative_path(&self) -> &str;

    /// The client wants to add a chunk to a fixed index file at a certain position.
    fn add_fixed_data(&mut self, index: u64, chunk: &FixedChunk) -> Result<()>;

    /// The client wants to add a chunks to a dynamic index file.
    fn add_dynamic_data(&mut self, chunk: &DynamicChunk) -> Result<()>;

    /// This notifies the handler that the backup has finished successfully. This should commit the
    /// data to the store for good. After this the client will receive an "ok".
    ///
    /// If the Drop handler gets called before this method, the backup was aborted due to an error
    /// or the client disconnected unexpectedly, in which case cleanup of temporary files should be
    /// performed.
    fn finish(&mut self) -> Result<()>;
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct BackupId(backup_type::Type, String, i64);

/// Associates a socket with the server side of the backup protocol.
/// The communcation channel should be `Read + Write` and may be non-blocking (provided it
/// correctly returns `io::ErrorKind::WouldBlock`).
/// The handler has to implement the `HandleClient` trait to provide callbacks used while
/// communicating with the client.
pub struct Connection<S, H>
where
    S: Read + Write,
    H: HandleClient,
{
    handler: H,
    common: common::Connection<S>,

    // states:

    // If this is set we are currently transferring our hash list to the client:
    hash_list: Option<(
        u8, // data stream ID
        Box<dyn ChunkList>,
    )>,

    // currently active 'backups' (handlers for a specific BackupDir)
    backups: HashMap<BackupId, Box<dyn HandleBackup + Send>>,

    // currently active backup *file* streams
    backup_files: HashMap<u8, Box<dyn BackupFile + Send>>,
}

impl<S, H> Connection<S, H>
where
    S: Read + Write,
    H: HandleClient,
{
    pub fn new(socket: S, handler: H) -> Result<Self> {
        let mut me = Self {
            handler,
            common: common::Connection::new(socket),
            hash_list: None,
            backups: HashMap::new(),
            backup_files: HashMap::new(),
        };

        me.send_hello()?;
        Ok(me)
    }

    fn send_hello(&mut self) -> Result<()> {
        let mut packet = Packet::builder(0, PacketType::Hello);
        packet.write_data(server::Hello {
            magic: server::HELLO_MAGIC,
            version: server::PROTOCOL_VERSION,
        });
        self.common.queue_data(packet.finish())?;
        Ok(())
    }

    pub fn eof(&self) -> bool {
        self.common.eof
    }

    /// It is safe to clear the error after an `io::ErrorKind::Interrupted`.
    pub fn clear_err(&mut self) {
        self.common.clear_err()
    }

    pub fn main(&mut self) -> Result<()> {
        self.poll_read()?;
        self.poll_send()?;
        Ok(())
    }

    // If this returns an error it is considered fatal and the connection should be dropped!
    fn poll_read(&mut self) -> Result<()> {
        if self.common.eof {
            // polls after EOF are errors:
            bail!("client disconnected");
        }

        if !self.common.poll_read()? {
            // No data available
            if self.common.eof {
                bail!("client disconnected");
            }
            return Ok(());
        }

        // we received a packet, handle it:

        loop {
            use PacketType::*;
            match self.common.current_packet_type {
                GetHashList => self.hash_list_requested()?,
                UploadChunk => self.receive_chunk()?,
                CreateBackup => self.create_backup()?,
                BackupDataDynamic => self.backup_data_dynamic()?,
                BackupDataFixed => self.backup_data_fixed()?,
                BackupFinished => self.backup_finished()?,
                _ => bail!(
                    "client sent an unexpected packet of type {}",
                    self.common.current_packet_type as u32,
                ),
            };
            self.common.next()?;
            if !self.common.poll_read()? {
                break;
            }
        }

        Ok(())
    }

    fn poll_send(&mut self) -> Result<()> {
        if self.common.error {
            eprintln!("refusing to send datato client in error state");
            bail!("client is in error state");
        }

        if let Some(false) = self.common.poll_send()? {
            // send queue is not finished, don't add anything else...
            return Ok(());
        }

        // Queue has either finished or was empty, see if we should enqueue more data:
        if self.hash_list.is_some() {
            return self.send_hash_list();
        }
        Ok(())
    }

    fn hash_list_requested(&mut self) -> Result<()> {
        // Verify protocol: GetHashList is an empty packet.
        let request = self.common.read_unaligned_data::<client::GetHashList>(0)?;
        self.common
            .assert_size(mem::size_of_val(&request) + request.name_length as usize)?;
        let name_bytes = &self.common.packet_data()[mem::size_of_val(&request)..];
        let name = std::str::from_utf8(name_bytes)?;

        // We support only one active hash list stream:
        if self.hash_list.is_some() {
            return self.respond_error(ErrorId::Busy);
        }

        self.hash_list = Some((
            self.common.current_packet.id,
            self.handler.get_chunk_list(name)?,
        ));

        Ok(())
    }

    fn send_hash_list(&mut self) -> Result<()> {
        loop {
            let (stream_id, hash_iter) = self.hash_list.as_mut().unwrap();

            let max_chunks_per_packet = (MAX_PACKET_SIZE as usize - mem::size_of::<Packet>())
                / mem::size_of::<FixedChunk>();

            let mut packet = Packet::builder(*stream_id, PacketType::HashListPart);
            packet.reserve(mem::size_of::<FixedChunk>() * max_chunks_per_packet);

            let mut count = 0;
            for _ in 0..max_chunks_per_packet {
                let entry: &[u8; 32] = match hash_iter.next() {
                    Ok(Some(entry)) => entry,
                    Ok(None) => break,
                    Err(e) => {
                        eprintln!("error sending chunk list to client: {}", e);
                        continue;
                    }
                };

                packet.write_buf(entry);
                count += 1;
            }

            let can_send_more = self.common.queue_data(packet.finish())?;

            if count == 0 {
                // We just sent the EOF packet, clear our iterator state!
                self.hash_list = None;
                break;
            }

            if !can_send_more {
                break;
            }
        }
        Ok(())
    }

    fn respond_error(&mut self, kind: ErrorId) -> Result<()> {
        self.respond_value(PacketType::Error, kind)?;
        Ok(())
    }

    fn respond_value<T: Endian>(&mut self, pkttype: PacketType, data: T) -> Result<()> {
        let mut packet = Packet::builder(self.common.current_packet.id, pkttype);
        packet.write_data(data);
        self.common.queue_data(packet.finish())?;
        Ok(())
    }

    fn respond_empty(&mut self, pkttype: PacketType) -> Result<()> {
        self.common
            .queue_data(Packet::simple(self.common.current_packet.id, pkttype))?;
        Ok(())
    }

    fn respond_ok(&mut self) -> Result<()> {
        self.respond_empty(PacketType::Ok)
    }

    fn receive_chunk(&mut self) -> Result<()> {
        self.common
            .assert_atleast(mem::size_of::<client::UploadChunk>())?;
        let data = self.common.packet_data();
        let (hash, data) = data.split_at(mem::size_of::<FixedChunk>());
        if data.len() == 0 {
            bail!("received an empty chunk");
        }
        let entry = ChunkEntry::from_data(data);
        if entry.hash != hash {
            let cli_hash = crate::tools::digest_to_hex(hash);
            let data_hash = entry.digest_to_hex();
            bail!(
                "client claimed data with digest {} has digest {}",
                data_hash,
                cli_hash
            );
        }
        let _new = self.handler.upload_chunk(&entry, data)?;
        self.respond_ok()
    }

    fn create_backup(&mut self) -> Result<()> {
        if self
            .backup_files
            .contains_key(&self.common.current_packet.id)
        {
            bail!("stream id already in use...");
        }

        let create_msg = self.common.read_unaligned_data::<client::CreateBackup>(0)?;

        // simple data:
        let flags = create_msg.flags;
        let backup_type = create_msg.backup_type;
        let time = create_msg.timestamp as i64;

        // text comes from the payload data after the CreateBackup struct:
        let data = self.common.packet_data();
        let payload = &data[mem::size_of_val(&create_msg)..];

        // there must be exactly the ID and the file name in the payload:
        let id_len = create_msg.id_length as usize;
        let name_len = create_msg.name_length as usize;
        let expected_len = id_len + name_len;
        if payload.len() < expected_len {
            bail!("client sent incomplete CreateBackup request");
        } else if payload.len() > expected_len {
            bail!("client sent excess data in CreateBackup request");
        }

        // id and file name must be utf8:
        let id = std::str::from_utf8(&payload[0..id_len])
            .map_err(|e| format_err!("client-requested backup id is invalid: {}", e))?;
        let file_name = std::str::from_utf8(&payload[id_len..])
            .map_err(|e| format_err!("client-requested backup file name invalid: {}", e))?;

        // Sanity check dynamic vs fixed:
        let is_dynamic = (flags & backup_flags::DYNAMIC_CHUNKS) != 0;
        let file_size = match (is_dynamic, create_msg.file_size) {
            (false, size) => Some(size),
            (true, 0) => None,
            (true, _) => bail!("file size of dynamic streams must be zero"),
        };

        // search or create the handler:
        let hashmap_id = BackupId(backup_type, id.to_string(), time);
        let handle = match self.backups.entry(hashmap_id) {
            hash_map::Entry::Vacant(entry) => entry.insert(self.handler.create_backup(
                backup_type::id_to_name(backup_type)?,
                id,
                time,
                (flags & backup_flags::EXCL) != 0,
            )?),
            hash_map::Entry::Occupied(entry) => entry.into_mut(),
        };
        let file = handle.create_file(file_name, file_size, create_msg.chunk_size as usize)?;

        let mut response =
            Packet::builder(self.common.current_packet.id, PacketType::BackupCreated);
        let path = file.relative_path();
        if path.len() > 0xffff {
            bail!("path too long");
        }
        response
            .write_data(server::BackupCreated {
                path_length: path.len() as _,
            })
            .write_buf(path.as_bytes());
        self.common.queue_data(response.finish())?;

        self.backup_files
            .insert(self.common.current_packet.id, file);

        Ok(())
    }

    fn backup_data_dynamic(&mut self) -> Result<()> {
        let stream_id = self.common.current_packet.id;
        let file = self
            .backup_files
            .get_mut(&stream_id)
            .ok_or_else(|| format_err!("BackupDataDynamic for invalid stream id {}", stream_id))?;

        let mut data = self.common.packet_data();
        // Data consists of (offset: u64, hash: [u8; 32])
        let entry_len = mem::size_of::<DynamicChunk>();

        while data.len() >= entry_len {
            let mut entry = unsafe { ptr::read_unaligned(data.as_ptr() as *const DynamicChunk) };
            data = &data[entry_len..];

            entry.offset = entry.offset.from_le();
            file.add_dynamic_data(&entry)?;
        }

        if data.len() != 0 {
            bail!(
                "client sent excess data ({} bytes) after dynamic chunk indices!",
                data.len()
            );
        }

        Ok(())
    }

    fn backup_data_fixed(&mut self) -> Result<()> {
        let stream_id = self.common.current_packet.id;
        let file = self
            .backup_files
            .get_mut(&stream_id)
            .ok_or_else(|| format_err!("BackupDataFixed for invalid stream id {}", stream_id))?;

        let mut data = self.common.packet_data();
        // Data consists of (index: u64, hash: [u8; 32])
        #[repr(C, packed)]
        struct IndexedChunk {
            index: u64,
            digest: FixedChunk,
        }
        let entry_len = mem::size_of::<IndexedChunk>();

        while data.len() >= entry_len {
            let mut entry = unsafe { ptr::read_unaligned(data.as_ptr() as *const IndexedChunk) };
            data = &data[entry_len..];

            entry.index = entry.index.from_le();
            file.add_fixed_data(entry.index, &entry.digest)?;
        }

        if data.len() != 0 {
            bail!(
                "client sent excess data ({} bytes) after dynamic chunk indices!",
                data.len()
            );
        }

        Ok(())
    }

    fn backup_finished(&mut self) -> Result<()> {
        let stream_id = self.common.current_packet.id;
        let mut file = self
            .backup_files
            .remove(&stream_id)
            .ok_or_else(|| format_err!("BackupDataDynamic for invalid stream id {}", stream_id))?;
        file.finish()?;
        self.respond_ok()
    }
}
