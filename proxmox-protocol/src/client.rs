use std::borrow::Borrow;
use std::collections::hash_map;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::mem;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use endian_trait::Endian;
use failure::*;

use crate::common;
use crate::protocol::*;
use crate::tools::swapped_data_to_buf;
use crate::{ChunkEntry, FixedChunk, IndexType};

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(transparent)]
pub struct BackupStream(pub(crate) u8);

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(transparent)]
pub struct StreamId(pub(crate) u8);

impl From<BackupStream> for StreamId {
    fn from(v: BackupStream) -> Self {
        Self(v.0)
    }
}

struct BackupStreamData {
    id: u8,
    index_type: IndexType,
    pos: u64,
    path: Option<String>,
}

pub enum AckState {
    Waiting,      // no ack received yet.
    Received,     // already received an ack, but the user hasn't seen it yet.
    Ignore,       // client doesn't care.
    AwaitingData, // waiting for something other than an 'Ok' packet
}

pub struct Client<S>
where
    S: Read + Write,
{
    chunks: RwLock<HashSet<FixedChunk>>,
    common: common::Connection<S>,
    handshake_done: bool,

    cur_id: u8,
    free_ids: Vec<u8>,
    waiting_ids: HashMap<u8, AckState>,
    hash_download: Option<u8>,

    upload_chunk: Option<FixedChunk>,
    upload_id: u8,
    upload_pos: usize,
    upload_state: u8,

    streams: HashMap<u8, BackupStreamData>,
}

type Result<T> = std::result::Result<T, Error>;

impl<S> Client<S>
where
    S: Read + Write,
{
    pub fn new(socket: S) -> Self {
        Self {
            chunks: RwLock::new(HashSet::new()),
            common: common::Connection::new(socket),
            handshake_done: false,

            cur_id: 1,
            free_ids: Vec::new(),
            waiting_ids: HashMap::new(),
            hash_download: None,

            upload_state: 0,
            upload_pos: 0,
            upload_id: 0,
            upload_chunk: None,

            streams: HashMap::new(),
        }
    }

    pub fn eof(&self) -> bool {
        self.common.eof
    }

    pub fn error(&self) -> bool {
        self.common.error
    }

    /// It is safe to clear the error after an `io::ErrorKind::Interrupted`.
    pub fn clear_err(&mut self) {
        self.common.clear_err()
    }

    pub fn wait_for_handshake(&mut self) -> Result<bool> {
        if !self.handshake_done {
            self.poll_read(true)?;
        }
        Ok(self.handshake_done)
    }

    pub fn query_hashes(&mut self, file_name: &str) -> Result<()> {
        if self.hash_download.is_some() {
            bail!("hash query already in progress");
        }

        let id = self.next_id()?;
        let mut packet = Packet::builder(id, PacketType::GetHashList);
        packet
            .write_data(client::GetHashList {
                name_length: file_name.len() as u16,
            })
            .write_buf(file_name.as_bytes());
        self.common.queue_data(packet.finish())?;
        self.hash_download = Some(id);
        Ok(())
    }

    pub fn wait_for_hashes(&mut self) -> Result<bool> {
        while self.hash_download.is_some() {
            if !self.poll_read(true)? {
                break;
            }
        }
        Ok(self.hash_download.is_none())
    }

    fn chunk_read_lock(&self) -> Result<RwLockReadGuard<HashSet<FixedChunk>>> {
        self.chunks
            .read()
            .map_err(|_| format_err!("lock poisoned, disconnecting client..."))
    }

    pub fn is_chunk_available<T: Borrow<FixedChunk>>(&self, chunk: &T) -> bool {
        self.chunk_read_lock().unwrap().contains(chunk.borrow())
    }

    /// Attempts to upload a chunk. Returns an error state only on fatal errors. If the underlying
    /// writer returns a `WouldBlock` error this returns `None` and `continue_upload_chunk` has to
    /// be called until the chunk is uploaded completely. During this time no other operations
    /// should be performed on the this object!
    /// See `continue_upload_chunk()` for a description of the returned value.
    pub fn upload_chunk<T>(&mut self, info: &T, data: &[u8]) -> Result<Option<StreamId>>
    where
        T: Borrow<FixedChunk>,
    {
        if self.upload_chunk.is_some() {
            bail!("cannot concurrently upload multiple chunks");
        }

        self.upload_id = self.next_id()?;
        self.upload_chunk = Some(info.borrow().clone());
        self.next_upload_state(0);
        self.continue_upload_chunk(data)
    }

    fn next_upload_state(&mut self, state: u8) {
        self.upload_state = state;
        self.upload_pos = 0;
    }

    // This is split into a static method not borrowing self so the buffer can point into self...
    fn do_upload_write(
        writer: &mut common::Connection<S>,
        buf: &[u8],
    ) -> Result<Option<(usize, bool)>> {
        match writer.write_some(buf) {
            Ok(put) => Ok(Some((put, put == buf.len()))),
            Err(e) => {
                if e.kind() == std::io::ErrorKind::WouldBlock {
                    Ok(None)
                } else {
                    Err(e.into())
                }
            }
        }
    }

    fn after_upload_write(&mut self, put: Option<(usize, bool)>, next_state: u8) -> bool {
        match put {
            None => return false,
            Some((put, done)) => {
                if done {
                    self.next_upload_state(next_state);
                } else {
                    self.upload_pos += put;
                }
                done
            }
        }
    }

    fn upload_write(&mut self, buf: &[u8], next_state: u8) -> Result<bool> {
        let wrote = Self::do_upload_write(&mut self.common, buf)?;
        Ok(self.after_upload_write(wrote, next_state))
    }

    /// If an `upload_chunk()` call returned `Ok(false)` this needs to be used to complete the
    /// upload process as the chunk may have already been partially written to the socket.
    /// This function will return `Ok(false)` on `WouldBlock` errors just like `upload_chunk()`
    /// will, after which the caller should wait for the writer to become write-ready and then
    /// call this method again.
    /// Once the complete chunk packet has been written to the underlying socket, this returns a
    /// packet ID which can be waited upon for completion
    pub fn continue_upload_chunk(&mut self, data: &[u8]) -> Result<Option<StreamId>> {
        loop {
            match self.upload_state {
                // Writing the packet header:
                0 => {
                    let len = mem::size_of::<Packet>()
                        + mem::size_of::<client::UploadChunk>()
                        + data.len();
                    let packet = Packet {
                        id: self.upload_id,
                        pkttype: PacketType::UploadChunk as _,
                        length: len as _,
                    }
                    .to_le();
                    let buf = unsafe { swapped_data_to_buf(&packet) };
                    if !self.upload_write(&buf[self.upload_pos..], 1)? {
                        return Ok(None);
                    }
                }
                // Writing the hash:
                1 => {
                    let chunk = self.upload_chunk.as_ref().unwrap();
                    let buf = &chunk.0[self.upload_pos..];
                    let wrote = Self::do_upload_write(&mut self.common, buf)?;
                    if !self.after_upload_write(wrote, 2) {
                        return Ok(None);
                    }
                }
                // Writing the data:
                2 => {
                    if !self.upload_write(&data[self.upload_pos..], 3)? {
                        return Ok(None);
                    }
                }
                // Done:
                3 => {
                    self.upload_chunk = None;
                    self.expect_ok_for_id(self.upload_id);
                    return Ok(Some(StreamId(self.upload_id)));
                }
                n => bail!("bad chunk upload state: {}", n),
            }
        }
    }

    // generic data polling method, returns true if at least one packet was received
    pub fn poll_read(&mut self, one: bool) -> Result<bool> {
        if self.common.eof {
            // polls after EOF are errors:
            bail!("server disconnected");
        }

        if !self.common.poll_read()? {
            // On the client side we do not expect a server-side disconnect, so error out!
            if self.common.eof {
                bail!("server disconnected");
            }
            return Ok(false);
        }

        loop {
            match self.common.current_packet_type {
                PacketType::Ok => self.recv_ok()?,
                PacketType::Hello => self.recv_hello()?,
                PacketType::HashListPart => self.recv_hash_list()?,
                PacketType::BackupCreated => self.backup_created()?,
                _ => bail!(
                    "server sent an unexpected packet of type {}",
                    self.common.current_packet_type as u32,
                ),
            }
            self.common.next()?;
            if one || !self.common.poll_read()? {
                break;
            }
        }
        Ok(true)
    }

    // None => nothing was queued
    // Some(true) => queue finished
    // Some(false) => queue not finished
    pub fn poll_send(&mut self) -> Result<Option<bool>> {
        self.common.poll_send()
    }

    // private helpermethods

    fn next_id(&mut self) -> Result<u8> {
        if let Some(id) = self.free_ids.pop() {
            return Ok(id);
        }
        if self.cur_id < 0xff {
            self.cur_id += 1;
            return Ok(self.cur_id - 1);
        }
        bail!("too many concurrent transactions");
    }

    fn free_id(&mut self, id: u8) {
        self.free_ids.push(id);
    }

    fn expect_ok_for_id(&mut self, id: u8) {
        self.waiting_ids.insert(id, AckState::Waiting);
    }

    fn expect_response_for_id(&mut self, id: u8) {
        self.waiting_ids.insert(id, AckState::AwaitingData);
    }

    // Methods handling received packets:

    fn recv_hello(&mut self) -> Result<()> {
        let hello = self.common.read_unaligned_data::<server::Hello>(0)?;

        if hello.magic != server::HELLO_MAGIC {
            bail!("received an invalid hello packet");
        }

        let ver = hello.version;
        if ver != server::PROTOCOL_VERSION {
            bail!(
                "hello packet contained incompatible protocol version: {} (!= {})",
                ver,
                server::PROTOCOL_VERSION,
            );
        }

        self.handshake_done = true;
        self.free_id(0);
        Ok(())
    }

    fn chunk_write_lock(&self) -> Result<RwLockWriteGuard<HashSet<FixedChunk>>> {
        self.chunks
            .write()
            .map_err(|_| format_err!("lock poisoned, disconnecting client..."))
    }

    fn recv_hash_list(&mut self) -> Result<()> {
        let data = self.common.packet_data();
        if data.len() == 0 {
            // No more hashes, we're done
            self.hash_download = None;
            return Ok(());
        }

        if (data.len() % mem::size_of::<FixedChunk>()) != 0 {
            bail!("hash list contains invalid size");
        }

        let chunk_list: &[FixedChunk] = unsafe {
            std::slice::from_raw_parts(
                data.as_ptr() as *const FixedChunk,
                data.len() / mem::size_of::<FixedChunk>(),
            )
        };

        let mut my_chunks = self.chunk_write_lock()?;
        for c in chunk_list {
            eprintln!("Got chunk '{}'", c.digest_to_hex());
            my_chunks.insert(c.clone());
        }

        Ok(())
    }

    fn ack_id(&mut self, id: u8, data_packet: bool) -> Result<()> {
        use hash_map::Entry::*;

        match self.waiting_ids.entry(id) {
            Vacant(_) => bail!("received unexpected packet for transaction id {}", id),
            Occupied(mut entry) => match entry.get() {
                AckState::Ignore => {
                    entry.remove();
                }
                AckState::Received => bail!("duplicate Ack received for transaction id {}", id),
                AckState::Waiting => {
                    if data_packet {
                        bail!("received data packet while expecting simple Ok for {}", id);
                    }
                    *entry.get_mut() = AckState::Received;
                }
                AckState::AwaitingData => {
                    if !data_packet {
                        bail!(
                            "received empty Ok while waiting for data on stream id {}",
                            id
                        );
                    }
                    *entry.get_mut() = AckState::Received;
                }
            },
        }
        Ok(())
    }

    fn recv_ok(&mut self) -> Result<()> {
        self.ack_id(self.common.current_packet.id, false)
    }

    pub fn wait_for_id(&mut self, id: StreamId) -> Result<bool> {
        if !self.waiting_ids.contains_key(&id.0) {
            bail!("wait_for_id() called on unexpected id {}", id.0);
        }

        loop {
            if !self.poll_read(true)? {
                return Ok(false);
            }

            use hash_map::Entry::*;
            match self.waiting_ids.entry(id.0) {
                Vacant(_) => return Ok(true),
                Occupied(entry) => match entry.get() {
                    AckState::Received => {
                        entry.remove();
                        return Ok(true);
                    }
                    _ => continue,
                },
            }
        }
    }

    pub fn discard_id(&mut self, id: StreamId) -> Result<()> {
        use hash_map::Entry::*;
        match self.waiting_ids.entry(id.0) {
            Vacant(_) => bail!("discard_id called with unknown id {}", id.0),
            Occupied(mut entry) => match entry.get() {
                AckState::Ignore => (),
                AckState::Received => {
                    entry.remove();
                }
                AckState::Waiting | AckState::AwaitingData => {
                    *entry.get_mut() = AckState::Ignore;
                }
            },
        }
        Ok(())
    }

    pub fn create_backup(
        &mut self,
        index_type: IndexType,
        backup_type: &str,
        id: &str,
        timestamp: i64,
        file_name: &str,
        chunk_size: usize,
        file_size: Option<u64>,
        is_new: bool,
    ) -> Result<BackupStream> {
        let backup_type = backup_type::name_to_id(backup_type)?;

        if id.len() > 0xff {
            bail!("id too long");
        }

        if file_name.len() > 0xff {
            bail!("file name too long");
        }

        let mut flags: backup_flags::Type = 0;
        if is_new {
            flags |= backup_flags::EXCL;
        }
        if index_type == IndexType::Dynamic {
            flags |= backup_flags::DYNAMIC_CHUNKS;
            if file_size.is_some() {
                bail!("file size must be None on dynamic backup streams");
            }
        } else if file_size.is_none() {
            bail!("file size is mandatory for fixed backup streams");
        }

        let packet_id = self.next_id()?;
        let mut packet = Packet::builder(packet_id, PacketType::CreateBackup);
        packet
            .write_data(client::CreateBackup {
                backup_type,
                id_length: id.len() as _,
                timestamp: timestamp as u64,
                flags,
                name_length: file_name.len() as _,
                chunk_size: chunk_size as _,
                file_size: file_size.unwrap_or(0) as u64,
            })
            .write_buf(id.as_bytes())
            .write_buf(file_name.as_bytes());

        self.streams.insert(
            packet_id,
            BackupStreamData {
                id: packet_id,
                index_type,
                pos: 0,
                path: None,
            },
        );

        self.expect_response_for_id(packet_id);
        self.common.queue_data(packet.finish())?;
        Ok(BackupStream(packet_id))
    }

    fn backup_created(&mut self) -> Result<()> {
        let info = self
            .common
            .read_unaligned_data::<server::BackupCreated>(0)?;
        let data = &self.common.packet_data()[mem::size_of_val(&info)..];
        if data.len() != info.path_length as usize {
            bail!("backup-created packet has invalid length");
        }
        let name = std::str::from_utf8(data)?;
        let pkt_id = self.common.current_packet.id;
        self.streams
            .get_mut(&pkt_id)
            .ok_or_else(|| format_err!("BackupCreated response for invalid stream: {}", pkt_id))?
            .path = Some(name.to_string());
        self.ack_id(pkt_id, true)?;
        Ok(())
    }

    pub fn dynamic_chunk(&mut self, stream: BackupStream, entry: &ChunkEntry) -> Result<bool> {
        self.dynamic_data(stream, &entry.hash, entry.size)
    }

    pub fn dynamic_data<T: Borrow<FixedChunk>>(
        &mut self,
        stream: BackupStream,
        digest: &T,
        size: u64,
    ) -> Result<bool> {
        let data = self
            .streams
            .get_mut(&stream.0)
            .ok_or_else(|| format_err!("no such active backup stream"))?;

        if data.index_type != IndexType::Dynamic {
            bail!("dynamic_data called for stream of static chunks");
        }

        let mut packet = Packet::builder(data.id, PacketType::BackupDataDynamic);
        packet
            .write_data(data.pos as u64)
            .write_buf(&digest.borrow().0);
        data.pos += size;

        self.common.queue_data(packet.finish())
    }

    pub fn fixed_data<T: Borrow<FixedChunk>>(
        &mut self,
        stream: BackupStream,
        index: usize,
        digest: &T,
    ) -> Result<bool> {
        let data = self
            .streams
            .get_mut(&stream.0)
            .ok_or_else(|| format_err!("no such active backup stream"))?;

        if data.index_type != IndexType::Fixed {
            bail!("fixed_data called for stream of dynamic chunks");
        }

        let mut packet = Packet::builder(data.id, PacketType::BackupDataFixed);
        packet
            .write_data(index as u64)
            .write_buf(&digest.borrow().0);

        self.common.queue_data(packet.finish())
    }

    pub fn finish_backup(&mut self, stream: BackupStream) -> Result<(StreamId, String, bool)> {
        let path = self
            .streams
            .remove(&stream.0)
            .ok_or_else(|| format_err!("no such active backup stream"))?
            .path
            .unwrap_or_else(|| "<no remote name received>".to_string());
        let ack = self
            .common
            .queue_data(Packet::simple(stream.0, PacketType::BackupFinished))?;
        self.expect_ok_for_id(stream.0);
        Ok((StreamId(stream.0), path, ack))
    }
}
