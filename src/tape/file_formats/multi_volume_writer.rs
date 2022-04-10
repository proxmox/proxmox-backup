use anyhow::Error;

use proxmox_uuid::Uuid;

use pbs_tape::{MediaContentHeader, TapeWrite};

/// Writes data streams using multiple volumes
///
/// Note: We do not use this feature currently.
pub struct MultiVolumeWriter<'a> {
    writer: Option<Box<dyn TapeWrite + 'a>>,
    next_writer_fn: Box<dyn 'a + FnMut() -> Result<Box<dyn TapeWrite + 'a>, Error>>,
    got_leom: bool,
    finished: bool,
    wrote_header: bool,
    header: MediaContentHeader,
    header_data: Vec<u8>,
    bytes_written: usize, // does not include bytes from current writer
}

impl<'a> MultiVolumeWriter<'a> {
    /// Creates a new instance
    pub fn new(
        writer: Box<dyn TapeWrite + 'a>,
        content_magic: [u8; 8],
        header_data: Vec<u8>,
        next_writer_fn: Box<dyn 'a + FnMut() -> Result<Box<dyn TapeWrite + 'a>, Error>>,
    ) -> Self {
        let header = MediaContentHeader::new(content_magic, header_data.len() as u32);

        Self {
            writer: Some(writer),
            next_writer_fn,
            got_leom: false,
            finished: false,
            header,
            header_data,
            wrote_header: false,
            bytes_written: 0,
        }
    }

    /// Returns the cuntent Uuid with the current part number
    pub fn uuid_and_part_number(&self) -> (Uuid, usize) {
        (self.header.uuid.into(), self.header.part_number as usize)
    }
}

impl<'a> TapeWrite for MultiVolumeWriter<'a> {
    fn write_all(&mut self, buf: &[u8]) -> Result<bool, std::io::Error> {
        if self.finished {
            proxmox_lang::io_bail!("multi-volume writer already finished: internal error");
        }

        if self.got_leom {
            if !self.wrote_header {
                proxmox_lang::io_bail!(
                    "multi-volume writer: got LEOM before writing anything - internal error"
                );
            }
            let mut writer = match self.writer.take() {
                Some(writer) => writer,
                None => proxmox_lang::io_bail!("multi-volume writer: no writer  -internal error"),
            };
            self.bytes_written = writer.bytes_written();
            writer.finish(true)?;
        }

        if self.writer.is_none() {
            if self.header.part_number == u8::MAX {
                proxmox_lang::io_bail!("multi-volume writer: too many parts");
            }
            self.writer = Some((self.next_writer_fn)().map_err(|err| {
                proxmox_lang::io_format_err!("multi-volume get next volume failed: {}", err)
            })?);
            self.got_leom = false;
            self.wrote_header = false;
            self.header.part_number += 1;
        }

        let leom = match self.writer {
            None => unreachable!(),
            Some(ref mut writer) => {
                if !self.wrote_header {
                    writer.write_header(&self.header, &self.header_data)?;
                    self.wrote_header = true;
                }
                writer.write_all(buf)?
            }
        };

        if leom {
            self.got_leom = true;
        }

        Ok(false)
    }

    fn bytes_written(&self) -> usize {
        let mut bytes_written = self.bytes_written;
        if let Some(ref writer) = self.writer {
            bytes_written += writer.bytes_written();
        }
        bytes_written
    }

    fn finish(&mut self, incomplete: bool) -> Result<bool, std::io::Error> {
        if incomplete {
            proxmox_lang::io_bail!(
                "incomplete flag makes no sense for multi-volume stream: internal error"
            );
        }

        match self.writer.take() {
            None if self.finished => {
                proxmox_lang::io_bail!("multi-volume writer already finished: internal error")
            }
            None => Ok(false),
            Some(ref mut writer) => {
                self.finished = true;
                if !self.wrote_header {
                    writer.write_header(&self.header, &self.header_data)?;
                    self.wrote_header = true;
                }
                writer.finish(false)
            }
        }
    }

    fn logical_end_of_media(&self) -> bool {
        self.got_leom
    }
}
