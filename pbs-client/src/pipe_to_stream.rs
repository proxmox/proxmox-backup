// Implement simple flow control for h2 client
//
// See also: hyper/src/proto/h2/mod.rs

use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::{format_err, Error};
use bytes::Bytes;
use futures::{ready, Future};
use h2::SendStream;

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
    type Output = Result<(), Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.get_mut();

        if this.data.is_some() {
            // just reserve 1 byte to make sure there's some
            // capacity available. h2 will handle the capacity
            // management for the actual body chunk.
            this.body_tx.reserve_capacity(1);

            if this.body_tx.capacity() == 0 {
                loop {
                    match ready!(this.body_tx.poll_capacity(cx)) {
                        Some(Err(err)) => return Poll::Ready(Err(Error::from(err))),
                        Some(Ok(0)) => {}
                        Some(Ok(_)) => break,
                        None => return Poll::Ready(Err(format_err!("protocol canceled"))),
                    }
                }
            } else if let Poll::Ready(reset) = this.body_tx.poll_reset(cx) {
                return Poll::Ready(Err(match reset {
                    Ok(reason) => format_err!("stream received RST_STREAM: {:?}", reason),
                    Err(err) => Error::from(err),
                }));
            }

            this.body_tx
                .send_data(this.data.take().unwrap(), true)
                .map_err(Error::from)?;

            Poll::Ready(Ok(()))
        } else {
            if let Poll::Ready(reset) = this.body_tx.poll_reset(cx) {
                return Poll::Ready(Err(match reset {
                    Ok(reason) => format_err!("stream received RST_STREAM: {:?}", reason),
                    Err(err) => Error::from(err),
                }));
            }
            Poll::Ready(Ok(()))
        }
    }
}
