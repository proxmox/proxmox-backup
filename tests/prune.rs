use failure::*;
use std::path::PathBuf;

use proxmox_backup::backup::*;

fn get_prune_list(
    list: Vec<BackupInfo>,
    keep_last: Option<u64>,
    keep_daily: Option<u64>,
    keep_weekly: Option<u64>,
    keep_monthly: Option<u64>,
    keep_yearly: Option<u64>,
) -> Vec<PathBuf> {

   let mut prune_info = BackupGroup::compute_prune_info(
        list, keep_last, keep_daily, keep_weekly, keep_monthly, keep_yearly).unwrap();

    prune_info.reverse();
    
    prune_info
        .iter()
        .filter_map(|(info, keep)| {
            if *keep {
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

    let backup_dir = BackupDir::parse(snapshot).unwrap();

    let mut files = Vec::new();

    if !partial {
        files.push(String::from(MANIFEST_BLOB_NAME));
    }
    
    BackupInfo { backup_dir, files }
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
    let remove_list = get_prune_list(list, Some(4), None, None, None, None);
    let expect: Vec<PathBuf> = Vec::new();
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let remove_list = get_prune_list(list, Some(3), None, None, None, None);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let remove_list = get_prune_list(list, Some(2), None, None, None, None);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let remove_list = get_prune_list(list, Some(1), None, None, None, None);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    let list = orig_list.clone();
    let remove_list = get_prune_list(list, Some(0), None, None, None, None);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T12:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    // keep-last, keep-daily mixed
    let list = orig_list.clone();
    let remove_list = get_prune_list(list, Some(2), Some(2), None, None, None);
    let expect: Vec<PathBuf> = vec![];
    assert_eq!(remove_list, expect);

    // keep-daily test
    let list = orig_list.clone();
    let remove_list = get_prune_list(list, None, Some(3), None, None, None);
    let expect: Vec<PathBuf> = vec![PathBuf::from("host/elsa/2019-12-04T11:59:15Z")];
    assert_eq!(remove_list, expect);

    // keep-daily test
    let list = orig_list.clone();
    let remove_list = get_prune_list(list, None, Some(2), None, None, None);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    // keep-weekly
    let list = orig_list.clone();
    let remove_list = get_prune_list(list, None, None, Some(5), None, None);
    // all backup are within the same week, so we only keep a single file
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    // keep-daily + keep-weekly
    let list = orig_list.clone();
    let remove_list = get_prune_list(list, None, Some(1), Some(5), None, None);
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    // keep-monthly
    let list = orig_list.clone();
    let remove_list = get_prune_list(list, None, None, None, Some(6), None);
    // all backup are within the same month, so we only keep a single file
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    // keep-yearly
    let list = orig_list.clone();
    let remove_list = get_prune_list(list, None, None, None, None, Some(7));
    // all backup are within the same year, so we only keep a single file
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    // keep-weekly + keep-monthly + keep-yearly
    let list = orig_list.clone();
    let remove_list = get_prune_list(list, None, None, Some(5), Some(6), Some(7));
    // all backup are within one week, so we only keep a single file
    let expect: Vec<PathBuf> = vec![
        PathBuf::from("host/elsa/2019-12-02T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-03T11:59:15Z"),
        PathBuf::from("host/elsa/2019-12-04T11:59:15Z"),
    ];
    assert_eq!(remove_list, expect);

    Ok(())
}
