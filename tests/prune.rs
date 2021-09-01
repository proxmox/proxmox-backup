use anyhow::{Error};
use std::path::PathBuf;

use pbs_datastore::manifest::MANIFEST_BLOB_NAME;
use pbs_datastore::prune::{compute_prune_info, PruneOptions};
use pbs_datastore::{BackupDir, BackupInfo};

fn get_prune_list(
    list: Vec<BackupInfo>,
    return_kept: bool,
    options: &PruneOptions,
) -> Vec<PathBuf> {

    let mut prune_info = compute_prune_info(list, options).unwrap();

    prune_info.reverse();

    prune_info
        .iter()
        .filter_map(|(info, keep)| {
            if *keep != return_kept {
                None
            } else {
                Some(info.backup_dir.relative_path())
            }
        })
        .collect()
}

fn create_info(
    snapshot: &str,
    partial: bool,
) -> BackupInfo {

    let backup_dir: BackupDir = snapshot.parse().unwrap();

    let mut files = Vec::new();

    if !partial {
        files.push(String::from(MANIFEST_BLOB_NAME));
    }

    BackupInfo { backup_dir, files }
}

#[test]
fn test_prune_hourly() -> Result<(), Error> {

    let mut orig_list = Vec::new();

    orig_list.push(create_info("host/elsa/2019-11-15T09:39:15Z", false));
    orig_list.push(create_info("host/elsa/2019-11-15T10:49:15Z", false));
    orig_list.push(create_info("host/elsa/2019-11-15T10:59:15Z", false));
    orig_list.push(create_info("host/elsa/2019-11-15T11:39:15Z", false));
    orig_list.push(create_info("host/elsa/2019-11-15T11:49:15Z", false));
    orig_list.push(create_info("host/elsa/2019-11-15T11:59:15Z", false));

    let list = orig_list.clone();
    let options = PruneOptions::new().keep_hourly(Some(3));
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-11-15T10:49:15Z"),
        PathBuf::from("host/elsa/2019-11-15T11:39:15Z"),
        PathBuf::from("host/elsa/2019-11-15T11:49:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list;
    let options = PruneOptions::new().keep_hourly(Some(2));
    let remove_list = get_prune_list(list, true, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-11-15T10:59:15Z"),
        PathBuf::from("host/elsa/2019-11-15T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    Ok(())
}

    #[test]
fn test_prune_simple2() -> Result<(), Error> {

    let mut orig_list = Vec::new();

    orig_list.push(create_info("host/elsa/2018-11-15T11:59:15Z", false));
    orig_list.push(create_info("host/elsa/2019-11-15T11:59:15Z", false));
    orig_list.push(create_info("host/elsa/2019-11-21T11:59:15Z", false));
    orig_list.push(create_info("host/elsa/2019-11-22T11:59:15Z", false));
    orig_list.push(create_info("host/elsa/2019-11-29T11:59:15Z", false));
    orig_list.push(create_info("host/elsa/2019-12-01T11:59:15Z", false));
    orig_list.push(create_info("host/elsa/2019-12-02T11:59:15Z", false));
    orig_list.push(create_info("host/elsa/2019-12-03T11:59:15Z", false));
    orig_list.push(create_info("host/elsa/2019-12-04T11:59:15Z", false));

    let list = orig_list.clone();
    let options = PruneOptions::new().keep_daily(Some(1));
    let remove_list = get_prune_list(list, true, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let options = PruneOptions::new().keep_last(Some(1)).keep_daily(Some(1));
    let remove_list = get_prune_list(list, true, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let options = PruneOptions::new().keep_daily(Some(1)).keep_weekly(Some(1));
    let remove_list = get_prune_list(list, true, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-01T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let options = PruneOptions::new().keep_daily(Some(1)).keep_weekly(Some(1)).keep_monthly(Some(1));
    let remove_list = get_prune_list(list, true, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-11-22T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-01T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list;
    let options = PruneOptions::new().keep_monthly(Some(1)).keep_yearly(Some(1));
    let remove_list = get_prune_list(list, true, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2018-11-15T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    Ok(())
}

#[test]
fn test_prune_simple() -> Result<(), Error> {

    let mut orig_list = Vec::new();

    orig_list.push(create_info("host/elsa/2019-12-02T11:59:15Z", false));
    orig_list.push(create_info("host/elsa/2019-12-03T11:59:15Z", false));
    orig_list.push(create_info("host/elsa/2019-12-04T11:59:15Z", false));
    orig_list.push(create_info("host/elsa/2019-12-04T12:59:15Z", false));

    // keep-last tests

    let list = orig_list.clone();
    let options = PruneOptions::new().keep_last(Some(4));
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = Vec::new();
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let options = PruneOptions::new().keep_last(Some(3));
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let options = PruneOptions::new().keep_last(Some(2));
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let options = PruneOptions::new().keep_last(Some(1));
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let options = PruneOptions::new().keep_last(Some(0));
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T12:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    // keep-last, keep-daily mixed
    let list = orig_list.clone();
    let options = PruneOptions::new().keep_last(Some(2)).keep_daily(Some(2));
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![];
    assert_eq!(remove_list, expect);

    // keep-daily test
    let list = orig_list.clone();
    let options = PruneOptions::new().keep_daily(Some(3));
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![PathBuf::from("host/elsa/2019-12-04T11:59:15Z")];
    assert_eq!(remove_list, expect);

    // keep-daily test
    let list = orig_list.clone();
    let options = PruneOptions::new().keep_daily(Some(2));
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    // keep-weekly
    let list = orig_list.clone();
    let options = PruneOptions::new().keep_weekly(Some(5));
    let remove_list = get_prune_list(list, false, &options);
    // all backup are within the same week, so we only keep a single file
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    // keep-daily + keep-weekly
    let list = orig_list.clone();
    let options = PruneOptions::new().keep_daily(Some(1)).keep_weekly(Some(5));
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    // keep-monthly
    let list = orig_list.clone();
    let options = PruneOptions::new().keep_monthly(Some(6));
    let remove_list = get_prune_list(list, false, &options);
    // all backup are within the same month, so we only keep a single file
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    // keep-yearly
    let list = orig_list.clone();
    let options = PruneOptions::new().keep_yearly(Some(7));
    let remove_list = get_prune_list(list, false, &options);
    // all backup are within the same year, so we only keep a single file
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    // keep-weekly + keep-monthly + keep-yearly
    let list = orig_list;
    let options = PruneOptions::new().keep_weekly(Some(5)).keep_monthly(Some(6)).keep_yearly(Some(7));
    let remove_list = get_prune_list(list, false, &options);
    // all backup are within one week, so we only keep a single file
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    Ok(())
}
