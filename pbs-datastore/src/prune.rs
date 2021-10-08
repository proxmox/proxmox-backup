use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Error};

use pbs_api_types::PruneOptions;

use super::BackupInfo;

enum PruneMark { Keep, KeepPartial, Remove }

fn mark_selections<F: Fn(&BackupInfo) -> Result<String, Error>> (
    mark: &mut HashMap<PathBuf, PruneMark>,
    list: &[BackupInfo],
    keep: usize,
    select_id: F,
) -> Result<(), Error> {

    let mut include_hash = HashSet::new();

    let mut already_included = HashSet::new();
    for info in list {
        let backup_id = info.backup_dir.relative_path();
        if let Some(PruneMark::Keep) = mark.get(&backup_id) {
            let sel_id: String = select_id(&info)?;
            already_included.insert(sel_id);
        }
    }

    for info in list {
        let backup_id = info.backup_dir.relative_path();
        if mark.get(&backup_id).is_some() { continue; }
        let sel_id: String = select_id(&info)?;

        if already_included.contains(&sel_id) { continue; }

        if !include_hash.contains(&sel_id) {
            if include_hash.len() >= keep { break; }
            include_hash.insert(sel_id);
            mark.insert(backup_id, PruneMark::Keep);
        } else {
            mark.insert(backup_id, PruneMark::Remove);
        }
    }

    Ok(())
}

fn remove_incomplete_snapshots(
    mark: &mut HashMap<PathBuf, PruneMark>,
    list: &[BackupInfo],
) {

    let mut keep_unfinished = true;
    for info in list.iter() {
        // backup is considered unfinished if there is no manifest
        if info.is_finished() {
            // There is a new finished backup, so there is no need
            // to keep older unfinished backups.
            keep_unfinished = false;
        } else {
            let backup_id = info.backup_dir.relative_path();
            if keep_unfinished { // keep first unfinished
                mark.insert(backup_id, PruneMark::KeepPartial);
            } else {
                mark.insert(backup_id, PruneMark::Remove);
            }
            keep_unfinished = false;
        }
    }
}

pub fn keeps_something(options: &PruneOptions) -> bool {
    let mut keep_something = false;
    if let Some(count) = options.keep_last { if count > 0 { keep_something = true; } }
    if let Some(count) = options.keep_hourly { if count > 0 { keep_something = true; } }
    if let Some(count) = options.keep_daily { if count > 0 { keep_something = true; } }
    if let Some(count) = options.keep_weekly { if count > 0 { keep_something = true; } }
    if let Some(count) = options.keep_monthly { if count > 0 { keep_something = true; } }
    if let Some(count) = options.keep_yearly { if count > 0 { keep_something = true; } }
    keep_something
}

pub fn cli_options_string(options: &PruneOptions) -> String {
    let mut opts = Vec::new();

    if let Some(count) = options.keep_last {
        if count > 0 {
            opts.push(format!("--keep-last {}", count));
        }
    }
    if let Some(count) = options.keep_hourly {
        if count > 0 {
            opts.push(format!("--keep-hourly {}", count));
        }
    }
    if let Some(count) = options.keep_daily {
        if count > 0 {
            opts.push(format!("--keep-daily {}", count));
        }
    }
    if let Some(count) = options.keep_weekly {
        if count > 0 {
            opts.push(format!("--keep-weekly {}", count));
        }
    }
    if let Some(count) = options.keep_monthly {
        if count > 0 {
            opts.push(format!("--keep-monthly {}", count));
        }
    }
    if let Some(count) = options.keep_yearly {
        if count > 0 {
            opts.push(format!("--keep-yearly {}", count));
        }
    }

    opts.join(" ")
}

pub fn compute_prune_info(
    mut list: Vec<BackupInfo>,
    options: &PruneOptions,
) -> Result<Vec<(BackupInfo, bool)>, Error> {

    let mut mark = HashMap::new();

    BackupInfo::sort_list(&mut list, false);

    remove_incomplete_snapshots(&mut mark, &list);

    if let Some(keep_last) = options.keep_last {
        mark_selections(&mut mark, &list, keep_last as usize, |info| {
            Ok(info.backup_dir.backup_time_string().to_owned())
        })?;
    }

    use proxmox_time::strftime_local;

    if let Some(keep_hourly) = options.keep_hourly {
        mark_selections(&mut mark, &list, keep_hourly as usize, |info| {
            strftime_local("%Y/%m/%d/%H", info.backup_dir.backup_time()).map_err(Error::from)
        })?;
    }

    if let Some(keep_daily) = options.keep_daily {
        mark_selections(&mut mark, &list, keep_daily as usize, |info| {
            strftime_local("%Y/%m/%d", info.backup_dir.backup_time()).map_err(Error::from)
        })?;
    }

    if let Some(keep_weekly) = options.keep_weekly {
        mark_selections(&mut mark, &list, keep_weekly as usize, |info| {
            // Note: Use iso-week year/week here. This year number
            // might not match the calendar year number.
            strftime_local("%G/%V", info.backup_dir.backup_time()).map_err(Error::from)
        })?;
    }

    if let Some(keep_monthly) = options.keep_monthly {
        mark_selections(&mut mark, &list, keep_monthly as usize, |info| {
            strftime_local("%Y/%m", info.backup_dir.backup_time()).map_err(Error::from)
        })?;
    }

    if let Some(keep_yearly) = options.keep_yearly {
        mark_selections(&mut mark, &list, keep_yearly as usize, |info| {
            strftime_local("%Y", info.backup_dir.backup_time()).map_err(Error::from)
        })?;
    }

    let prune_info: Vec<(BackupInfo, bool)> = list.into_iter()
        .map(|info| {
            let backup_id = info.backup_dir.relative_path();
            let keep = match mark.get(&backup_id) {
                Some(PruneMark::Keep) => true,
                Some(PruneMark::KeepPartial) => true,
               _ => false,
            };
            (info, keep)
        })
        .collect();

    Ok(prune_info)
}
