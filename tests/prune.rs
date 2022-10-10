use std::path::PathBuf;

use anyhow::Error;

use pbs_api_types::PruneJobOptions;
use pbs_datastore::manifest::MANIFEST_BLOB_NAME;
use pbs_datastore::prune::compute_prune_info;
use pbs_datastore::{BackupDir, BackupInfo};

fn get_prune_list(
    list: Vec<BackupInfo>,
    return_kept: bool,
    options: &PruneJobOptions,
) -> Vec<PathBuf> {
    let mut prune_info = compute_prune_info(list, &options.keep).unwrap();

    prune_info.reverse();

    prune_info
        .iter()
        .filter_map(|(info, mark)| {
            if mark.keep() != return_kept {
                None
            } else {
                Some(info.backup_dir.relative_path())
            }
        })
        .collect()
}

fn create_info(snapshot: &str, partial: bool) -> BackupInfo {
    let backup_dir = BackupDir::new_test(snapshot.parse().unwrap());

    let mut files = Vec::new();

    if !partial {
        files.push(String::from(MANIFEST_BLOB_NAME));
    }

    BackupInfo {
        backup_dir,
        files,
        protected: false,
    }
}

fn create_info_protected(snapshot: &str, partial: bool) -> BackupInfo {
    let mut info = create_info(snapshot, partial);
    info.protected = true;
    info
}

#[test]
fn test_prune_protected() -> Result<(), Error> {
    let orig_list = vec![
        create_info_protected("host/elsa/2019-11-15T09:39:15Z", false),
        create_info("host/elsa/2019-11-15T10:39:15Z", false),
        create_info("host/elsa/2019-11-15T10:49:15Z", false),
        create_info_protected("host/elsa/2019-11-15T10:59:15Z", false),
    ];

    eprintln!("{:?}", orig_list);

    let mut options = PruneJobOptions::default();
    options.keep.keep_last = Some(1);
    let remove_list = get_prune_list(orig_list.clone(), false, &options);
    let expect: Vec<PathBuf> = vec![PathBuf::from("host/elsa/2019-11-15T10:39:15Z")];
    assert_eq!(remove_list, expect);

    let mut options = PruneJobOptions::default();
    options.keep.keep_hourly = Some(1);
    let remove_list = get_prune_list(orig_list, false, &options);
    let expect: Vec<PathBuf> = vec![PathBuf::from("host/elsa/2019-11-15T10:39:15Z")];
    assert_eq!(remove_list, expect);
    Ok(())
}

#[test]
fn test_prune_hourly() -> Result<(), Error> {
    let orig_list = vec![
        create_info("host/elsa/2019-11-15T09:39:15Z", false),
        create_info("host/elsa/2019-11-15T10:49:15Z", false),
        create_info("host/elsa/2019-11-15T10:59:15Z", false),
        create_info("host/elsa/2019-11-15T11:39:15Z", false),
        create_info("host/elsa/2019-11-15T11:49:15Z", false),
        create_info("host/elsa/2019-11-15T11:59:15Z", false),
    ];

    let list = orig_list.clone();
    let mut options = PruneJobOptions::default();
    options.keep.keep_hourly = Some(3);
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-11-15T10:49:15Z"),
        PathBuf::from("host/elsa/2019-11-15T11:39:15Z"),
        PathBuf::from("host/elsa/2019-11-15T11:49:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list;
    let mut options = PruneJobOptions::default();
    options.keep.keep_hourly = Some(2);
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
    let orig_list = vec![
        create_info("host/elsa/2018-11-15T11:59:15Z", false),
        create_info("host/elsa/2019-11-15T11:59:15Z", false),
        create_info("host/elsa/2019-11-21T11:59:15Z", false),
        create_info("host/elsa/2019-11-22T11:59:15Z", false),
        create_info("host/elsa/2019-11-29T11:59:15Z", false),
        create_info("host/elsa/2019-12-01T11:59:15Z", false),
        create_info("host/elsa/2019-12-02T11:59:15Z", false),
        create_info("host/elsa/2019-12-03T11:59:15Z", false),
        create_info("host/elsa/2019-12-04T11:59:15Z", false),
    ];

    let list = orig_list.clone();
    let mut options = PruneJobOptions::default();
    options.keep.keep_daily = Some(1);
    let remove_list = get_prune_list(list, true, &options);
    let expect: Vec<PathBuf> = vec![PathBuf::from("host/elsa/2019-12-04T11:59:15Z")];
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let mut options = PruneJobOptions::default();
    options.keep.keep_last = Some(1);
    options.keep.keep_daily = Some(1);
    let remove_list = get_prune_list(list, true, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let mut options = PruneJobOptions::default();
    options.keep.keep_daily = Some(1);
    options.keep.keep_weekly = Some(1);
    let remove_list = get_prune_list(list, true, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-01T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let mut options = PruneJobOptions::default();
    options.keep.keep_daily = Some(1);
    options.keep.keep_weekly = Some(1);
    options.keep.keep_monthly = Some(1);
    let remove_list = get_prune_list(list, true, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-11-22T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-01T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list;
    let mut options = PruneJobOptions::default();
    options.keep.keep_monthly = Some(1);
    options.keep.keep_yearly = Some(1);
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
    let orig_list = vec![
        create_info("host/elsa/2019-12-02T11:59:15Z", false),
        create_info("host/elsa/2019-12-03T11:59:15Z", false),
        create_info("host/elsa/2019-12-04T11:59:15Z", false),
        create_info("host/elsa/2019-12-04T12:59:15Z", false),
    ];

    // keep-last tests

    let list = orig_list.clone();
    let mut options = PruneJobOptions::default();
    options.keep.keep_last = Some(4);
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = Vec::new();
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let mut options = PruneJobOptions::default();
    options.keep.keep_last = Some(3);
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![PathBuf::from("host/elsa/2019-12-02T11:59:15Z")];
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let mut options = PruneJobOptions::default();
    options.keep.keep_last = Some(2);
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let mut options = PruneJobOptions::default();
    options.keep.keep_last = Some(1);
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let mut options = PruneJobOptions::default();
    options.keep.keep_last = Some(0);
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
    let mut options = PruneJobOptions::default();
    options.keep.keep_last = Some(2);
    options.keep.keep_daily = Some(2);
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![];
    assert_eq!(remove_list, expect);

    // keep-daily test
    let list = orig_list.clone();
    let mut options = PruneJobOptions::default();
    options.keep.keep_daily = Some(3);
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![PathBuf::from("host/elsa/2019-12-04T11:59:15Z")];
    assert_eq!(remove_list, expect);

    // keep-daily test
    let list = orig_list.clone();
    let mut options = PruneJobOptions::default();
    options.keep.keep_daily = Some(2);
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    // keep-weekly
    let list = orig_list.clone();
    let mut options = PruneJobOptions::default();
    options.keep.keep_weekly = Some(5);
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
    let mut options = PruneJobOptions::default();
    options.keep.keep_daily = Some(1);
    options.keep.keep_weekly = Some(5);
    let remove_list = get_prune_list(list, false, &options);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    // keep-monthly
    let list = orig_list.clone();
    let mut options = PruneJobOptions::default();
    options.keep.keep_monthly = Some(6);
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
    let mut options = PruneJobOptions::default();
    options.keep.keep_yearly = Some(7);
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
    let mut options = PruneJobOptions::default();
    options.keep.keep_weekly = Some(5);
    options.keep.keep_monthly = Some(6);
    options.keep.keep_yearly = Some(7);
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
