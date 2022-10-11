// Tape inventory tests
//
// # cargo test --release tape::test::inventory

use anyhow::{bail, Error};
use std::path::PathBuf;

use proxmox_uuid::Uuid;

use pbs_api_types::{MediaLocation, MediaStatus};

use crate::tape::{file_formats::MediaSetLabel, Inventory};

fn create_testdir(name: &str) -> Result<PathBuf, Error> {
    let mut testdir: PathBuf = String::from("./target/testout").into();
    testdir.push(std::module_path!());
    testdir.push(name);

    let _ = std::fs::remove_dir_all(&testdir);
    let _ = std::fs::create_dir_all(&testdir);

    Ok(testdir)
}

#[test]
fn test_media_state_db() -> Result<(), Error> {
    let testdir = create_testdir("test_media_state_db")?;

    let mut inventory = Inventory::load(&testdir)?;

    let uuid1: Uuid = inventory.generate_free_tape("tape1", 0);

    assert_eq!(
        inventory.status_and_location(&uuid1),
        (MediaStatus::Unknown, MediaLocation::Offline)
    );

    inventory.set_media_status_full(&uuid1)?;

    assert_eq!(
        inventory.status_and_location(&uuid1),
        (MediaStatus::Full, MediaLocation::Offline)
    );

    inventory.set_media_location_vault(&uuid1, "Office2")?;
    assert_eq!(
        inventory.status_and_location(&uuid1),
        (
            MediaStatus::Full,
            MediaLocation::Vault(String::from("Office2"))
        )
    );

    inventory.set_media_location_offline(&uuid1)?;
    assert_eq!(
        inventory.status_and_location(&uuid1),
        (MediaStatus::Full, MediaLocation::Offline)
    );

    inventory.set_media_status_damaged(&uuid1)?;
    assert_eq!(
        inventory.status_and_location(&uuid1),
        (MediaStatus::Damaged, MediaLocation::Offline)
    );

    inventory.clear_media_status(&uuid1)?;
    assert_eq!(
        inventory.status_and_location(&uuid1),
        (MediaStatus::Unknown, MediaLocation::Offline)
    );

    Ok(())
}

#[test]
fn test_list_pool_media() -> Result<(), Error> {
    let testdir = create_testdir("test_list_pool_media")?;
    let mut inventory = Inventory::load(&testdir)?;

    let ctime = 0;

    let _tape1_uuid = inventory.generate_free_tape("tape1", ctime);
    let tape2_uuid = inventory.generate_assigned_tape("tape2", "p1", ctime);

    let set1 = MediaSetLabel::with_data("p1", Uuid::generate(), 0, ctime, None);

    let tape3_uuid = inventory.generate_used_tape("tape3", set1.clone(), ctime);

    let list = inventory.list_pool_media("nonexistent_pool");
    assert_eq!(list.len(), 0);

    let list = inventory.list_pool_media("p1");
    assert_eq!(list.len(), 2);

    let tape2 = list
        .iter()
        .find(|media_id| media_id.label.uuid == tape2_uuid)
        .unwrap();
    assert!(tape2.media_set_label.is_none());

    let tape3 = list
        .iter()
        .find(|media_id| media_id.label.uuid == tape3_uuid)
        .unwrap();
    match tape3.media_set_label {
        None => bail!("missing media set label"),
        Some(ref set) => {
            assert_eq!(set.seq_nr, 0);
            assert_eq!(set.uuid, set1.uuid);
        }
    }
    Ok(())
}

#[test]
fn test_media_set_simple() -> Result<(), Error> {
    let testdir = create_testdir("test_media_set_simple")?;
    let mut inventory = Inventory::load(&testdir)?;

    let ctime = 0;

    let sl1 = MediaSetLabel::with_data("p1", Uuid::generate(), 0, ctime + 10, None);
    let sl2 = MediaSetLabel::with_data("p1", sl1.uuid.clone(), 1, ctime + 20, None);
    let sl3 = MediaSetLabel::with_data("p1", sl1.uuid.clone(), 2, ctime + 30, None);

    let tape1_uuid = inventory.generate_used_tape("tape1", sl1.clone(), 0);
    let tape2_uuid = inventory.generate_used_tape("tape2", sl2, 0);
    let tape3_uuid = inventory.generate_used_tape("tape3", sl3, 0);

    // generate incomplete media set in pool p2
    let sl4 = MediaSetLabel::with_data("p2", Uuid::generate(), 1, ctime + 40, None);
    let tape4_uuid = inventory.generate_used_tape("tape4", sl4.clone(), 0);

    let media_list = inventory.list_pool_media("p1");
    assert_eq!(media_list.len(), 3);

    let media_list = inventory.list_pool_media("p2");
    assert_eq!(media_list.len(), 1);

    // reload, the do more tests

    let inventory = Inventory::load(&testdir)?;

    // test pool p1

    let media_set = inventory.compute_media_set_members(&sl1.uuid)?;
    assert_eq!(media_set.uuid(), &sl1.uuid);

    let media_list = media_set.media_list();
    assert_eq!(media_list.len(), 3);

    assert_eq!(media_list[0], Some(tape1_uuid));
    assert_eq!(media_list[1], Some(tape2_uuid));
    assert_eq!(media_list[2], Some(tape3_uuid));

    // test media set start time
    assert_eq!(inventory.media_set_start_time(&sl1.uuid), Some(ctime + 10));

    // test pool p2
    let media_set = inventory.compute_media_set_members(&sl4.uuid)?;
    assert_eq!(media_set.uuid(), &sl4.uuid);

    let media_list = media_set.media_list();
    assert_eq!(media_list.len(), 2);

    assert_eq!(media_list[0], None);
    assert_eq!(media_list[1], Some(tape4_uuid));

    // start time for incomplete set must be None
    assert_eq!(inventory.media_set_start_time(&sl4.uuid), None);

    Ok(())
}

#[test]
fn test_latest_media_set() -> Result<(), Error> {
    let testdir = create_testdir("test_latest_media_set")?;

    let insert_tape = |inventory: &mut Inventory, pool, label, seq_nr, ctime| -> Uuid {
        let sl = MediaSetLabel::with_data(pool, Uuid::generate(), seq_nr, ctime, None);
        let uuid = sl.uuid.clone();
        inventory.generate_used_tape(label, sl, 0);
        uuid
    };

    let check_latest = |inventory: &Inventory, pool: &str, label: &str| {
        let latest_set = inventory.latest_media_set(pool).unwrap();
        let set = inventory.compute_media_set_members(&latest_set).unwrap();
        let media_list = set.media_list();
        assert_eq!(media_list.iter().filter(|s| s.is_some()).count(), 1);
        let media_uuid = media_list
            .iter()
            .find(|s| s.is_some())
            .unwrap()
            .clone()
            .unwrap();
        let media = inventory.lookup_media(&media_uuid).unwrap();
        assert_eq!(media.label.label_text, label);
    };

    let mut inventory = Inventory::load(&testdir)?;

    let ctime = 0;

    // test 3 sets with different start times

    insert_tape(&mut inventory, "p1", "p1tape1", 0, ctime + 10);
    insert_tape(&mut inventory, "p1", "p1tape2", 0, ctime + 20);
    insert_tape(&mut inventory, "p1", "p1tape3", 0, ctime + 30);

    check_latest(&inventory, "p1", "p1tape3");

    // test 2 sets with same start times, should fail

    insert_tape(&mut inventory, "p2", "p2tape1", 0, ctime + 10);
    insert_tape(&mut inventory, "p2", "p2tape2", 0, ctime + 10);

    assert_eq!(inventory.latest_media_set("p2"), None);

    // test with incomplete sets

    insert_tape(&mut inventory, "p3", "p3tape1", 5, ctime + 50);
    insert_tape(&mut inventory, "p3", "p3tape2", 1, ctime + 10);
    insert_tape(&mut inventory, "p3", "p3tape3", 0, ctime + 20);

    check_latest(&inventory, "p3", "p3tape1");

    Ok(())
}
