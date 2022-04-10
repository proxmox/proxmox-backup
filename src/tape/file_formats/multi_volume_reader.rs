use std::io::Read;

use anyhow::{bail, Error};

use proxmox_io::ReadExt;

use pbs_tape::{MediaContentHeader, TapeRead};

/// Read multi volume data streams written by `MultiVolumeWriter`
///
/// Note: We do not use this feature currently.
pub struct MultiVolumeReader<'a> {
    reader: Option<Box<dyn TapeRead + 'a>>,
    next_reader_fn: Box<dyn 'a + FnMut() -> Result<Box<dyn TapeRead + 'a>, Error>>,
    complete: bool,
    header: MediaContentHeader,
}

impl<'a> MultiVolumeReader<'a> {
    /// Creates a new instance
    pub fn new(
        reader: Box<dyn TapeRead + 'a>,
        header: MediaContentHeader,
        next_reader_fn: Box<dyn 'a + FnMut() -> Result<Box<dyn TapeRead + 'a>, Error>>,
    ) -> Result<Self, Error> {
        if header.part_number != 0 {
            bail!(
                "MultiVolumeReader::new - got wrong header part_number ({} != 0)",
                header.part_number
            );
        }

        Ok(Self {
            reader: Some(reader),
            next_reader_fn,
            complete: false,
            header,
        })
    }
}

impl<'a> Read for MultiVolumeReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        if self.complete {
            return Ok(0);
        }

        if self.reader.is_none() {
            let mut reader = (self.next_reader_fn)()
                .map_err(|err| proxmox_lang::io_format_err!("multi-volume next failed: {}", err))?;

            proxmox_lang::try_block!({
                let part_header: MediaContentHeader = unsafe { reader.read_le_value()? };
                self.reader = Some(reader);

                if part_header.uuid != self.header.uuid {
                    proxmox_lang::io_bail!("got wrong part uuid");
                }
                if part_header.content_magic != self.header.content_magic {
                    proxmox_lang::io_bail!("got wrong part content magic");
                }

                let expect_part_number = self.header.part_number + 1;

                if part_header.part_number != expect_part_number {
                    proxmox_lang::io_bail!(
                        "got wrong part number ({} != {})",
                        part_header.part_number,
                        expect_part_number
                    );
                }

                self.header.part_number = expect_part_number;

                Ok(())
            })
            .map_err(|err| {
                proxmox_lang::io_format_err!("multi-volume read content header failed: {}", err)
            })?;
        }

        match self.reader {
            None => unreachable!(),
            Some(ref mut reader) => match reader.read(buf) {
                Ok(0) => {
                    if reader.is_incomplete()? {
                        self.reader = None;
                        self.read(buf)
                    } else {
                        self.reader = None;
                        self.complete = true;
                        Ok(0)
                    }
                }
                Ok(n) => Ok(n),
                Err(err) => Err(err),
            },
        }
    }
}
