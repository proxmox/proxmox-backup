use anyhow::{bail, Error};

use proxmox::tools::Uuid;

use crate::{
    tape::{
        MediaCatalog,
        MediaSetCatalog,
    },
};

/// Helper to build and query sets of catalogs
///
/// Similar to MediaSetCatalog, but allows to modify the last catalog.
pub struct CatalogSet {
    // read only part
    pub media_set_catalog: MediaSetCatalog,
    // catalog to modify (latest in  set)
    pub catalog: Option<MediaCatalog>,
}

impl CatalogSet {

    /// Create empty instance
    pub fn new() -> Self {
        Self {
            media_set_catalog: MediaSetCatalog::new(),
            catalog: None,
        }
    }

    /// Add catalog to the read-only set
    pub fn append_read_only_catalog(&mut self, catalog: MediaCatalog) -> Result<(), Error> {
        self.media_set_catalog.append_catalog(catalog)
    }

    /// Test if the catalog already contains a snapshot
    pub fn contains_snapshot(&self, store: &str, snapshot: &str) -> bool {
        if let Some(ref catalog) = self.catalog {
            if catalog.contains_snapshot(store, snapshot) {
                return true;
            }
        }
        self.media_set_catalog.contains_snapshot(store, snapshot)
    }

    /// Test if the catalog already contains a chunk
    pub fn contains_chunk(&self, store: &str, digest: &[u8;32]) -> bool {
        if let Some(ref catalog) = self.catalog {
            if catalog.contains_chunk(store, digest) {
                return true;
            }
        }
        self.media_set_catalog.contains_chunk(store, digest)
    }

    /// Add a new catalog, move the old on to the read-only set
    pub fn append_catalog(&mut self, new_catalog: MediaCatalog) -> Result<(), Error> {

        // append current catalog to read-only set
        if let Some(catalog) = self.catalog.take() {
            self.media_set_catalog.append_catalog(catalog)?;
        }

        // remove read-only version from set (in case it is there)
        self.media_set_catalog.remove_catalog(&new_catalog.uuid());

        self.catalog = Some(new_catalog);

        Ok(())
    }

    /// Register a snapshot
    pub fn register_snapshot(
        &mut self,
        uuid: Uuid, // Uuid form MediaContentHeader
        file_number: u64,
        store: &str,
        snapshot: &str,
    )  -> Result<(), Error> {
        match self.catalog {
            Some(ref mut catalog) => {
                catalog.register_snapshot(uuid, file_number, store, snapshot)?;
            }
            None => bail!("no catalog loaded - internal error"),
        }
        Ok(())
    }

    /// Register a chunk archive
    pub fn register_chunk_archive(
        &mut self,
        uuid: Uuid, // Uuid form MediaContentHeader
        file_number: u64,
        store: &str,
        chunk_list: &[[u8; 32]],
    ) -> Result<(), Error> {
        match self.catalog {
            Some(ref mut catalog) => {
                catalog.start_chunk_archive(uuid, file_number, store)?;
                for digest in chunk_list {
                    catalog.register_chunk(digest)?;
                }
                catalog.end_chunk_archive()?;
            }
            None => bail!("no catalog loaded - internal error"),
        }
        Ok(())
    }

    /// Commit the catalog changes
    pub fn commit(&mut self) -> Result<(), Error> {
        if let Some(ref mut catalog) = self.catalog {
            catalog.commit()?;
        }
        Ok(())
    }
}
