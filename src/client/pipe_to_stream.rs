// Implement simple flow control for h2 client
//
// See also: hyper/src/proto/h2/mod.rs

use failure::*;

use futures::{try_ready, Async, Future, Poll};
use h2::{SendStream};
use bytes::Bytes;

pub struct PipeToSendStream {
    body_tx: SendStream<Bytes>,
    data: Option<Bytes>,
}

impl PipeToSendStream {
    pub fn new(data: Bytes, tx: SendStream<Bytes>) -> PipeToSendStream {
        PipeToSendStream {
            body_tx: tx,
            data: Some(data),
        }
    }
}

impl Future for PipeToSendStream {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            if self.data != None {
                // we don't have the next chunk of data yet, so just reserve 1 byte to make
                // sure there's some capacity available. h2 will handle the capacity management
                // for the actual body chunk.
                self.body_tx.reserve_capacity(1);

                if self.body_tx.capacity() == 0 {
                    loop {
                        match try_ready!(self.body_tx.poll_capacity().map_err(Error::from)) {
                            Some(0) => {}
                            Some(_) => break,
                            None => return Err(format_err!("protocol canceled")),
                        }
                    }
                } else {
                    if let Async::Ready(reason) = self.body_tx.poll_reset().map_err(Error::from)? {
                        return Err(format_err!("stream received RST_STREAM: {:?}", reason));
                    }
                }

                self.body_tx
                    .send_data(self.data.take().unwrap(), true)
                    .map_err(Error::from)?;

                return Ok(Async::Ready(()));

            } else {
                if let Async::Ready(reason) = self.body_tx.poll_reset().map_err(Error::from)? {
                    return Err(format_err!("stream received RST_STREAM: {:?}", reason));
                }
                return Ok(Async::Ready(()));
            }
        }
    }
}
