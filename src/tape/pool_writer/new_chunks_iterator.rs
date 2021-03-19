use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use anyhow::{format_err, Error};

use crate::{
    backup::{
        DataStore,
        DataBlob,
    },
    tape::{
        CatalogSet,
        SnapshotReader,

    },
};

/// Chunk iterator which use a separate thread to read chunks
///
/// The iterator skips duplicate chunks and chunks already in the
/// catalog.
pub struct NewChunksIterator {
    rx: std::sync::mpsc::Receiver<Result<Option<([u8; 32], DataBlob)>, Error>>,
}

impl NewChunksIterator {

    /// Creates the iterator, spawning a new thread
    ///
    /// Make sure to join() the returnd thread handle.
    pub fn spawn(
        datastore: Arc<DataStore>,
        snapshot_reader: Arc<Mutex<SnapshotReader>>,
        catalog_set: Arc<Mutex<CatalogSet>>,
    ) -> Result<(std::thread::JoinHandle<()>, Self), Error> {

        let (tx, rx) = std::sync::mpsc::sync_channel(3);

        let reader_thread = std::thread::spawn(move || {

            let snapshot_reader = snapshot_reader.lock().unwrap();

            let mut chunk_index: HashSet<[u8;32]> = HashSet::new();

            let datastore_name = snapshot_reader.datastore_name();

            let result: Result<(), Error> = proxmox::try_block!({

                let mut chunk_iter = snapshot_reader.chunk_iterator()?;

                loop {
                    let digest = match chunk_iter.next() {
                        None => {
                            tx.send(Ok(None)).unwrap();
                            break;
                        }
                        Some(digest) => digest?,
                    };

                    if chunk_index.contains(&digest) {
                        continue;
                    }

                    if catalog_set.lock().unwrap().contains_chunk(&datastore_name, &digest) {
                        continue;
                    };

                    let blob = datastore.load_chunk(&digest)?;
                    //println!("LOAD CHUNK {}", proxmox::tools::digest_to_hex(&digest));
                    tx.send(Ok(Some((digest, blob)))).unwrap();

                    chunk_index.insert(digest);
                }

                Ok(())
            });
            if let Err(err) = result {
                tx.send(Err(err)).unwrap();
            }
        });

        Ok((reader_thread, Self { rx }))
    }
}

// We do not use Receiver::into_iter(). The manual implementation
// returns a simpler type.
impl Iterator for NewChunksIterator {
    type Item = Result<([u8; 32], DataBlob), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.rx.recv() {
            Ok(Ok(None)) => None,
            Ok(Ok(Some((digest, blob)))) => Some(Ok((digest, blob))),
            Ok(Err(err)) => Some(Err(err)),
            Err(_) => Some(Err(format_err!("reader thread failed"))),
        }
    }
}
