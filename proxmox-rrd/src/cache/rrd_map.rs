use std::path::Path;
use std::sync::Arc;
use std::collections::HashMap;

use anyhow::{bail, Error};

use proxmox::tools::fs::create_path;

use crate::rrd::{CF, DST, RRD};

use super::CacheConfig;

pub struct RRDMap {
    config: Arc<CacheConfig>,
    map: HashMap<String, RRD>,
    load_rrd_cb: fn(path: &Path, rel_path: &str, dst: DST) -> RRD,
}

impl RRDMap {

    pub(crate) fn new(
        config: Arc<CacheConfig>,
        load_rrd_cb: fn(path: &Path, rel_path: &str, dst: DST) -> RRD,
    ) -> Self {
        Self {
            config,
            map: HashMap::new(),
            load_rrd_cb,
        }
    }

    pub fn update(
        &mut self,
        rel_path: &str,
        time: f64,
        value: f64,
        dst: DST,
        new_only: bool,
    ) -> Result<(), Error> {
        if let Some(rrd) = self.map.get_mut(rel_path) {
            if !new_only || time > rrd.last_update() {
                rrd.update(time, value);
            }
        } else {
            let mut path = self.config.basedir.clone();
            path.push(rel_path);
            create_path(
                path.parent().unwrap(),
                Some(self.config.dir_options.clone()),
                Some(self.config.dir_options.clone()),
            )?;

            let mut rrd = (self.load_rrd_cb)(&path, rel_path, dst);

            if !new_only || time > rrd.last_update() {
                rrd.update(time, value);
            }
            self.map.insert(rel_path.to_string(), rrd);
        }
        Ok(())
    }

    pub fn flush_rrd_files(&self) -> Result<usize, Error> {
        let mut rrd_file_count = 0;

        let mut errors = 0;
        for (rel_path, rrd) in self.map.iter() {
            rrd_file_count += 1;

            let mut path = self.config.basedir.clone();
            path.push(&rel_path);

            if let Err(err) = rrd.save(&path, self.config.file_options.clone()) {
                errors += 1;
                log::error!("unable to save {:?}: {}", path, err);
            }
        }

        if errors != 0 {
            bail!("errors during rrd flush - unable to commit rrd journal");
        }

        Ok(rrd_file_count)
    }

    pub fn extract_cached_data(
        &self,
        base: &str,
        name: &str,
        cf: CF,
        resolution: u64,
        start: Option<u64>,
        end: Option<u64>,
    ) -> Result<Option<(u64, u64, Vec<Option<f64>>)>, Error> {
        match self.map.get(&format!("{}/{}", base, name)) {
            Some(rrd) => Ok(Some(rrd.extract_data(cf, resolution, start, end)?)),
            None => Ok(None),
        }
    }
}
