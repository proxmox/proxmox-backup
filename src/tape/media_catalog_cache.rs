use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{bail, format_err, Error};

use proxmox_sys::fs::CreateOptions;

use crate::tape::{MediaCatalog, MediaId};

/// Returns a list of (store, snapshot) for a given MediaId
///
/// To speedup things for large catalogs, we cache the list of
/// snapshots into a separate file.
pub fn media_catalog_snapshot_list<P: AsRef<Path>>(
    base_path: P,
    media_id: &MediaId,
) -> Result<Vec<(String, String)>, Error> {
    let uuid = &media_id.label.uuid;

    let mut cache_path = base_path.as_ref().to_owned();
    cache_path.push(uuid.to_string());
    let mut catalog_path = cache_path.clone();
    cache_path.set_extension("index");
    catalog_path.set_extension("log");

    let stat = match nix::sys::stat::stat(&catalog_path) {
        Ok(stat) => stat,
        Err(err) => bail!("unable to stat media catalog {:?} - {}", catalog_path, err),
    };

    let cache_id = format!(
        "{:016X}-{:016X}-{:016X}",
        stat.st_ino, stat.st_size as u64, stat.st_mtime as u64
    );

    match std::fs::OpenOptions::new().read(true).open(&cache_path) {
        Ok(file) => {
            let mut list = Vec::new();
            let file = BufReader::new(file);
            let mut lines = file.lines();
            match lines.next() {
                Some(Ok(id)) => {
                    if id != cache_id {
                        // cache is outdated - rewrite
                        return write_snapshot_cache(&base_path, media_id, &cache_path, &cache_id);
                    }
                }
                _ => bail!("unable to read catalog cache firstline {:?}", cache_path),
            }

            for line in lines {
                let mut line = line?;

                let idx = line
                    .find(':')
                    .ok_or_else(|| format_err!("invalid line format (no store found)"))?;

                let snapshot = line.split_off(idx + 1);
                line.truncate(idx);
                list.push((line, snapshot));
            }

            Ok(list)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            write_snapshot_cache(base_path, media_id, &cache_path, &cache_id)
        }
        Err(err) => bail!("unable to open catalog cache - {}", err),
    }
}

fn write_snapshot_cache<P: AsRef<Path>>(
    base_path: P,
    media_id: &MediaId,
    cache_path: &Path,
    cache_id: &str,
) -> Result<Vec<(String, String)>, Error> {
    // open normal catalog and write cache
    let catalog = MediaCatalog::open(base_path, media_id, false, false)?;

    let mut data = String::new();
    data.push_str(cache_id);
    data.push('\n');

    let mut list = Vec::new();
    for (store, content) in catalog.content() {
        for snapshot in content.snapshot_index.keys() {
            list.push((store.to_string(), snapshot.to_string()));
            data.push_str(store);
            data.push(':');
            data.push_str(snapshot);
            data.push('\n');
        }
    }

    let backup_user = pbs_config::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);
    let options = CreateOptions::new()
        .perm(mode)
        .owner(backup_user.uid)
        .group(backup_user.gid);

    proxmox_sys::fs::replace_file(cache_path, data.as_bytes(), options, false)?;

    Ok(list)
}
