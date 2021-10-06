use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::{RwLock};

use anyhow::{format_err, Error};

use proxmox::tools::fs::{create_path, CreateOptions};

use crate::{RRDMode, RRDTimeFrameResolution};

use super::*;

/// RRD cache - keep RRD data in RAM, but write updates to disk
///
/// This cache is designed to run as single instance (no concurrent
/// access from other processes).
pub struct RRDCache {
    basedir: PathBuf,
    file_options: CreateOptions,
    dir_options: CreateOptions,
    cache: RwLock<HashMap<String, RRD>>,
}

impl RRDCache {

    /// Creates a new instance
    pub fn new<P: AsRef<Path>>(
        basedir: P,
        file_options: Option<CreateOptions>,
        dir_options: Option<CreateOptions>,
    ) -> Self {
        let basedir = basedir.as_ref().to_owned();
        Self {
            basedir,
            file_options: file_options.unwrap_or_else(|| CreateOptions::new()),
            dir_options: dir_options.unwrap_or_else(|| CreateOptions::new()),
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Create rrdd stat dir with correct permission
    pub fn create_rrdb_dir(&self) -> Result<(), Error> {

        create_path(&self.basedir, Some(self.dir_options.clone()), Some(self.file_options.clone()))
            .map_err(|err: Error| format_err!("unable to create rrdb stat dir - {}", err))?;

        Ok(())
    }

    /// Update data in RAM and write file back to disk (if `save` is set)
    pub fn update_value(
        &self,
        rel_path: &str,
        value: f64,
        dst: DST,
        save: bool,
    ) -> Result<(), Error> {

        let mut path = self.basedir.clone();
        path.push(rel_path);

        create_path(path.parent().unwrap(), Some(self.dir_options.clone()), Some(self.file_options.clone()))?;

        let mut map = self.cache.write().unwrap();
        let now = proxmox::tools::time::epoch_f64();

        if let Some(rrd) = map.get_mut(rel_path) {
            rrd.update(now, value);
            if save { rrd.save(&path, self.file_options.clone())?; }
        } else {
            let mut rrd = match RRD::load(&path) {
                Ok(rrd) => rrd,
                Err(err) => {
                    if err.kind() != std::io::ErrorKind::NotFound {
                        eprintln!("overwriting RRD file {:?}, because of load error: {}", path, err);
                    }
                    RRD::new(dst)
                },
            };
            rrd.update(now, value);
            if save {
                rrd.save(&path, self.file_options.clone())?;
            }
            map.insert(rel_path.into(), rrd);
        }

        Ok(())
    }

    /// Extract data from cached RRD
    pub fn extract_cached_data(
        &self,
        base: &str,
        name: &str,
        now: f64,
        timeframe: RRDTimeFrameResolution,
        mode: RRDMode,
    ) -> Option<(u64, u64, Vec<Option<f64>>)> {

        let map = self.cache.read().unwrap();

        match map.get(&format!("{}/{}", base, name)) {
            Some(rrd) => Some(rrd.extract_data(now, timeframe, mode)),
            None => None,
        }
    }
}
