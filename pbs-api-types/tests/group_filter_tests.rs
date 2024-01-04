use pbs_api_types::{BackupGroup, BackupType, GroupFilter};
use std::str::FromStr;

#[test]
fn test_no_filters() {
    let group_filters = vec![];

    let do_backup = [
        "vm/101", "vm/102", "vm/103", "vm/104", "vm/105", "vm/106", "vm/107", "vm/108", "vm/109",
    ];

    for id in do_backup {
        assert!(BackupGroup::new(BackupType::Vm, id).apply_filters(&group_filters));
    }
}

#[test]
fn test_include_filters() {
    let group_filters = vec![GroupFilter::from_str("regex:.*10[2-8]").unwrap()];

    let do_backup = [
        "vm/102", "vm/103", "vm/104", "vm/105", "vm/106", "vm/107", "vm/108",
    ];

    let dont_backup = ["vm/101", "vm/109"];

    for id in do_backup {
        assert!(BackupGroup::new(BackupType::Vm, id).apply_filters(&group_filters));
    }

    for id in dont_backup {
        assert!(!BackupGroup::new(BackupType::Vm, id).apply_filters(&group_filters));
    }
}

#[test]
fn test_exclude_filters() {
    let group_filters = [
        GroupFilter::from_str("exclude:regex:.*10[1-3]").unwrap(),
        GroupFilter::from_str("exclude:regex:.*10[5-7]").unwrap(),
    ];

    let do_backup = ["vm/104", "vm/108", "vm/109"];

    let dont_backup = ["vm/101", "vm/102", "vm/103", "vm/105", "vm/106", "vm/107"];

    for id in do_backup {
        assert!(BackupGroup::new(BackupType::Vm, id).apply_filters(&group_filters));
    }
    for id in dont_backup {
        assert!(!BackupGroup::new(BackupType::Vm, id).apply_filters(&group_filters));
    }
}

#[test]
fn test_include_and_exclude_filters() {
    let group_filters = [
        GroupFilter::from_str("exclude:regex:.*10[1-3]").unwrap(),
        GroupFilter::from_str("regex:.*10[2-8]").unwrap(),
        GroupFilter::from_str("exclude:regex:.*10[5-7]").unwrap(),
    ];

    let do_backup = ["vm/104", "vm/108"];

    let dont_backup = [
        "vm/101", "vm/102", "vm/103", "vm/105", "vm/106", "vm/107", "vm/109",
    ];

    for id in do_backup {
        assert!(BackupGroup::new(BackupType::Vm, id).apply_filters(&group_filters));
    }

    for id in dont_backup {
        assert!(!BackupGroup::new(BackupType::Vm, id).apply_filters(&group_filters));
    }
}
