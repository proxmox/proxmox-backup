use std::io::{self, Read, Write};
use std::mem;
use std::ptr;

use failure::*;

use endian_trait::Endian;

use crate::protocol::*;

type Result<T> = std::result::Result<T, Error>;

pub(crate) struct Connection<S>
where
    S: Read + Write,
{
    socket: S,
    pub buffer: Vec<u8>,
    pub current_packet: Packet,
    pub current_packet_type: PacketType,
    pub error: bool,
    pub eof: bool,
    upload_queue: Option<(Vec<u8>, usize)>,
}

impl<S> Connection<S>
where
    S: Read + Write,
{
    pub fn new(socket: S) -> Self {
        Self {
            socket,
            buffer: Vec::new(),
            current_packet: unsafe { mem::zeroed() },
            current_packet_type: PacketType::Error,
            error: false,
            eof: false,
            upload_queue: None,
        }
    }

    pub fn write_some(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.socket.write(buf)
    }

    /// It is safe to clear the error after an `io::ErrorKind::Interrupted`.
    pub fn clear_err(&mut self) {
        self.error = false;
    }

    // None => nothing was queued
    // Some(true) => queue finished
    // Some(false) => queue not finished
    pub fn poll_send(&mut self) -> Result<Option<bool>> {
        if let Some((ref data, ref mut pos)) = self.upload_queue {
            loop {
                match self.socket.write(&data[*pos..]) {
                    Ok(put) => {
                        *pos += put;
                        if *pos == data.len() {
                            self.upload_queue = None;
                            return Ok(Some(true));
                        }
                        // Keep writing
                        continue;
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        return Ok(Some(false));
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        } else {
            Ok(None)
        }
    }

    // Returns true when the data was also sent out, false if the queue is now full.
    // For now we only allow a single dataset to be queued at once.
    pub fn queue_data(&mut self, buf: Vec<u8>) -> Result<bool> {
        if self.upload_queue.is_some() {
            bail!("upload queue clash");
        }

        self.upload_queue = Some((buf, 0));

        match self.poll_send()? {
            None => unreachable!(), // We literally just set self.upload_queue to Some(value)
            Some(v) => Ok(v),
        }
    }

    // Returns 'true' if there's data available, 'false' if there isn't (if the
    // underlying reader returned `WouldBlock` or the `read()` was short).
    // Other errors are propagated.
    pub fn poll_read(&mut self) -> Result<bool> {
        if self.eof {
            return Ok(false);
        }

        if self.error {
            eprintln!("refusing to read from a client in error state");
            bail!("client is in error state");
        }

        match self.poll_data_do() {
            Ok(has_packet) => Ok(has_packet),
            Err(e) => {
                // To support AsyncRead/AsyncWrite we do not enter a failed
                // state when we read from a non-blocking source which fails
                // with WouldBlock.
                if let Some(ioe) = e.downcast_ref::<std::io::Error>() {
                    if ioe.kind() == io::ErrorKind::WouldBlock {
                        return Ok(false);
                    }
                }
                self.error = true;
                Err(e)
            }
        }
    }

    fn poll_data_do(&mut self) -> Result<bool> {
        if !self.read_packet()? {
            return Ok(false);
        }

        if self.current_packet.length > MAX_PACKET_SIZE {
            bail!("client tried to send a huge packet");
        }

        if !self.fill_packet()? {
            return Ok(false);
        }

        Ok(true)
    }

    pub fn packet_length(&self) -> usize {
        self.current_packet.length as usize
    }

    pub fn packet_data(&self) -> &[u8] {
        let beg = mem::size_of::<Packet>();
        let end = self.packet_length();
        &self.buffer[beg..end]
    }

    pub fn next(&mut self) -> Result<bool> {
        let pktlen = self.packet_length();
        unsafe {
            if self.buffer.len() != pktlen {
                std::ptr::copy_nonoverlapping(
                    &self.buffer[pktlen],
                    &mut self.buffer[0],
                    self.buffer.len() - pktlen,
                );
            }
            self.buffer.set_len(self.buffer.len() - pktlen);
        }
        self.poll_data_do()
    }

    // NOTE: After calling this you must `self.buffer.set_len()` when done!
    #[must_use]
    fn buffer_set_min_size(&mut self, size: usize) -> usize {
        if self.buffer.capacity() < size {
            self.buffer.reserve(size - self.buffer.len());
        }
        let start = self.buffer.len();
        unsafe {
            self.buffer.set_len(size);
        }
        start
    }

    fn fill_buffer(&mut self, size: usize) -> Result<bool> {
        if self.buffer.len() >= size {
            return Ok(true);
        }
        let mut filled = self.buffer_set_min_size(size);
        loop {
            // We don't use read_exact to not block too long or busy-read on nonblocking sockets...
            match self.socket.read(&mut self.buffer[filled..]) {
                Ok(got) => {
                    if got == 0 {
                        self.eof = true;
                        unsafe {
                            self.buffer.set_len(filled);
                        }
                        return Ok(false);
                    }
                    filled += got;
                    if filled >= size {
                        unsafe {
                            self.buffer.set_len(filled);
                        }
                        return Ok(true);
                    }
                    // reloop
                }
                Err(e) => {
                    unsafe {
                        self.buffer.set_len(filled);
                    }
                    return Err(e.into());
                }
            }
        }
    }

    fn read_packet_do(&mut self) -> Result<bool> {
        if !self.fill_buffer(mem::size_of::<Packet>())? {
            return Ok(false);
        }

        self.current_packet = self.read_unaligned::<Packet>(0)?.from_le();

        self.current_packet_type = match PacketType::try_from(self.current_packet.pkttype) {
            Some(t) => t,
            None => bail!("unexpected packet type"),
        };

        let length = self.current_packet.length;
        if (length as usize) < mem::size_of::<Packet>() {
            bail!("received packet of bad length ({})", length);
        }

        Ok(true)
    }

    fn read_packet(&mut self) -> Result<bool> {
        match self.read_packet_do() {
            Ok(b) => Ok(b),
            Err(e) => {
                if let Some(ioe) = e.downcast_ref::<std::io::Error>() {
                    if ioe.kind() == io::ErrorKind::WouldBlock {
                        return Ok(false);
                    }
                }
                Err(e)
            }
        }
    }

    fn read_unaligned<T: Endian>(&self, offset: usize) -> Result<T> {
        if offset + mem::size_of::<T>() > self.buffer.len() {
            bail!("buffer underrun");
        }
        Ok(unsafe { ptr::read_unaligned(&self.buffer[offset] as *const _ as *const T) }.from_le())
    }

    pub fn read_unaligned_data<T: Endian>(&self, offset: usize) -> Result<T> {
        self.read_unaligned(offset + mem::size_of::<Packet>())
    }

    fn fill_packet(&mut self) -> Result<bool> {
        self.fill_buffer(self.current_packet.length as usize)
    }

    // convenience helpers:

    pub fn assert_size(&self, size: usize) -> Result<()> {
        if self.packet_data().len() != size {
            bail!(
                "protocol error: invalid packet size (type {})",
                self.current_packet.pkttype,
            );
        }
        Ok(())
    }

    pub fn assert_atleast(&self, size: usize) -> Result<()> {
        if self.packet_data().len() < size {
            bail!(
                "protocol error: invalid packet size (type {})",
                self.current_packet.pkttype,
            );
        }
        Ok(())
    }
}
