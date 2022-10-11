// Tape Media Pool tests - test current_set_usable() function
//
// # cargo test --release tape::test::current_set_usable

use anyhow::Error;
use std::path::PathBuf;

use proxmox_uuid::Uuid;

use pbs_api_types::{MediaSetPolicy, RetentionPolicy};

use crate::tape::{file_formats::MediaSetLabel, Inventory, MediaPool};

fn create_testdir(name: &str) -> Result<PathBuf, Error> {
    let mut testdir: PathBuf = String::from("./target/testout").into();
    testdir.push(std::module_path!());
    testdir.push(name);

    let _ = std::fs::remove_dir_all(&testdir);
    let _ = std::fs::create_dir_all(&testdir);

    Ok(testdir)
}

#[test]
fn test_current_set_usable_1() -> Result<(), Error> {
    let testdir = create_testdir("test_current_set_usable_1")?;

    // pool without any media

    let pool = MediaPool::new(
        "p1",
        &testdir,
        MediaSetPolicy::AlwaysCreate,
        RetentionPolicy::KeepForever,
        None,
        None,
        false,
    )?;

    assert!(!pool.current_set_usable()?);

    Ok(())
}

#[test]
fn test_current_set_usable_2() -> Result<(), Error> {
    let testdir = create_testdir("test_current_set_usable_2")?;

    let ctime = 0;

    let mut inventory = Inventory::load(&testdir)?;

    inventory.generate_assigned_tape("tape1", "p1", ctime);

    // pool with one free media
    let pool = MediaPool::new(
        "p1",
        &testdir,
        MediaSetPolicy::AlwaysCreate,
        RetentionPolicy::KeepForever,
        None,
        None,
        false,
    )?;

    assert!(!pool.current_set_usable()?);

    Ok(())
}

#[test]
fn test_current_set_usable_3() -> Result<(), Error> {
    let testdir = create_testdir("test_current_set_usable_3")?;

    let ctime = 0;

    let mut inventory = Inventory::load(&testdir)?;

    let sl1 = MediaSetLabel::with_data("p1", Uuid::generate(), 0, ctime, None);

    inventory.generate_used_tape("tape1", sl1, ctime); // Note: Tape is offline

    // pool with one media in current set, only use online media
    let pool = MediaPool::new(
        "p1",
        &testdir,
        MediaSetPolicy::AlwaysCreate,
        RetentionPolicy::KeepForever,
        Some(String::from("changer1")),
        None,
        false,
    )?;

    assert!(!pool.current_set_usable()?);

    Ok(())
}

#[test]
fn test_current_set_usable_4() -> Result<(), Error> {
    let testdir = create_testdir("test_current_set_usable_4")?;

    let ctime = 0;

    let mut inventory = Inventory::load(&testdir)?;

    let sl1 = MediaSetLabel::with_data("p1", Uuid::generate(), 0, ctime, None);

    inventory.generate_used_tape("tape1", sl1, ctime); // Note: Tape is offline

    // pool with one media in current set, use offline media
    let pool = MediaPool::new(
        "p1",
        &testdir,
        MediaSetPolicy::AlwaysCreate,
        RetentionPolicy::KeepForever,
        None,
        None,
        false,
    )?;

    assert!(pool.current_set_usable()?);

    Ok(())
}

#[test]
fn test_current_set_usable_5() -> Result<(), Error> {
    let testdir = create_testdir("test_current_set_usable_5")?;

    let ctime = 0;

    let mut inventory = Inventory::load(&testdir)?;

    let sl1 = MediaSetLabel::with_data("p1", Uuid::generate(), 0, ctime, None);
    let sl2 = MediaSetLabel::with_data("p1", sl1.uuid.clone(), 1, ctime + 1, None);

    inventory.generate_used_tape("tape1", sl1, ctime);
    inventory.generate_used_tape("tape2", sl2, ctime);

    // pool with two media in current set
    let pool = MediaPool::new(
        "p1",
        &testdir,
        MediaSetPolicy::AlwaysCreate,
        RetentionPolicy::KeepForever,
        None,
        None,
        false,
    )?;

    assert!(pool.current_set_usable()?);

    Ok(())
}

#[test]
fn test_current_set_usable_6() -> Result<(), Error> {
    let testdir = create_testdir("test_current_set_usable_6")?;

    let ctime = 0;

    let mut inventory = Inventory::load(&testdir)?;

    let sl2 = MediaSetLabel::with_data("p1", Uuid::generate(), 1, ctime + 1, None);

    inventory.generate_used_tape("tape2", sl2, ctime);

    // pool with incomplete current set
    let pool = MediaPool::new(
        "p1",
        &testdir,
        MediaSetPolicy::AlwaysCreate,
        RetentionPolicy::KeepForever,
        None,
        None,
        false,
    )?;

    assert!(pool.current_set_usable().is_err());

    Ok(())
}

#[test]
fn test_current_set_usable_7() -> Result<(), Error> {
    let testdir = create_testdir("test_current_set_usable_7")?;

    let ctime = 0;

    let mut inventory = Inventory::load(&testdir)?;

    let sl1 = MediaSetLabel::with_data("p1", Uuid::generate(), 0, ctime, None);
    let sl2 = MediaSetLabel::with_data("p1", sl1.uuid.clone(), 1, ctime + 1, None);

    // generate damaged tape
    let tape1_uuid = inventory.generate_used_tape("tape1", sl1, ctime);
    inventory.set_media_status_damaged(&tape1_uuid)?;

    inventory.generate_used_tape("tape2", sl2, ctime);

    // pool with one two media in current set, one set to damaged
    let pool = MediaPool::new(
        "p1",
        &testdir,
        MediaSetPolicy::AlwaysCreate,
        RetentionPolicy::KeepForever,
        None,
        None,
        false,
    )?;

    assert!(pool.current_set_usable().is_err());

    Ok(())
}
