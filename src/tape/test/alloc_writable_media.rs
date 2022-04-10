// Tape Media Pool tests - test allow_ritable_media() function
//
// # cargo test --release tape::test::alloc_writable_media

use anyhow::Error;
use std::path::PathBuf;

use pbs_api_types::{MediaSetPolicy, RetentionPolicy};

use crate::tape::{Inventory, MediaPool};

fn create_testdir(name: &str) -> Result<PathBuf, Error> {
    let mut testdir: PathBuf = String::from("./target/testout").into();
    testdir.push(std::module_path!());
    testdir.push(name);

    let _ = std::fs::remove_dir_all(&testdir);
    let _ = std::fs::create_dir_all(&testdir);

    Ok(testdir)
}

#[test]
fn test_alloc_writable_media_1() -> Result<(), Error> {
    let testdir = create_testdir("test_alloc_writable_media_1")?;

    let mut ctime = 0;

    let mut pool = MediaPool::new(
        "p1",
        &testdir,
        MediaSetPolicy::ContinueCurrent,
        RetentionPolicy::KeepForever,
        None,
        None,
        false,
    )?;

    ctime += 10;

    pool.start_write_session(ctime, false)?;

    // no media in pool
    assert!(pool.alloc_writable_media(ctime).is_err());

    Ok(())
}

#[test]
fn test_alloc_writable_media_2() -> Result<(), Error> {
    let testdir = create_testdir("test_alloc_writable_media_2")?;

    let mut inventory = Inventory::load(&testdir)?;

    // tape1: free, assigned to pool
    let tape1_uuid = inventory.generate_assigned_tape("tape1", "p1", 0);

    let mut pool = MediaPool::new(
        "p1",
        &testdir,
        MediaSetPolicy::ContinueCurrent,
        RetentionPolicy::KeepForever,
        None,
        None,
        false,
    )?;

    let ctime = 10;

    pool.start_write_session(ctime, false)?;

    // use free media
    assert_eq!(pool.alloc_writable_media(ctime)?, tape1_uuid);
    // call again, media is still writable
    assert_eq!(pool.alloc_writable_media(ctime)?, tape1_uuid);

    // mark tape1 a Full
    pool.set_media_status_full(&tape1_uuid)?;

    // next call fail because there is no free media
    assert!(pool.alloc_writable_media(ctime).is_err());

    Ok(())
}

#[test]
fn test_alloc_writable_media_3() -> Result<(), Error> {
    let testdir = create_testdir("test_alloc_writable_media_3")?;

    let mut inventory = Inventory::load(&testdir)?;

    // tape1: free, assigned to pool
    let tape1_uuid = inventory.generate_assigned_tape("tape1", "p1", 0);
    // tape2: free, assigned to pool
    let tape2_uuid = inventory.generate_assigned_tape("tape1", "p1", 1);

    let mut pool = MediaPool::new(
        "p1",
        &testdir,
        MediaSetPolicy::ContinueCurrent,
        RetentionPolicy::KeepForever,
        None,
        None,
        false,
    )?;

    let mut ctime = 10;

    pool.start_write_session(ctime, false)?;

    // use free media
    assert_eq!(pool.alloc_writable_media(ctime)?, tape1_uuid);
    // call again, media is still writable
    ctime += 1;
    assert_eq!(pool.alloc_writable_media(ctime)?, tape1_uuid);

    // mark tape1 a Full
    pool.set_media_status_full(&tape1_uuid)?;

    // use next free media
    ctime += 1;
    assert_eq!(pool.alloc_writable_media(ctime)?, tape2_uuid);

    // mark tape2 a Full
    pool.set_media_status_full(&tape2_uuid)?;

    // next call fail because there is no free media
    ctime += 1;
    assert!(pool.alloc_writable_media(ctime).is_err());

    Ok(())
}

#[test]
fn test_alloc_writable_media_4() -> Result<(), Error> {
    let testdir = create_testdir("test_alloc_writable_media_4")?;

    let mut inventory = Inventory::load(&testdir)?;

    // tape1: free, assigned to pool
    let tape1_uuid = inventory.generate_assigned_tape("tape1", "p1", 0);

    let mut pool = MediaPool::new(
        "p1",
        &testdir,
        MediaSetPolicy::AlwaysCreate,
        RetentionPolicy::ProtectFor("12s".parse()?),
        None,
        None,
        false,
    )?;

    let start_time = 10;

    pool.start_write_session(start_time, false)?;

    // use free media
    assert_eq!(pool.alloc_writable_media(start_time)?, tape1_uuid);
    // call again, media is still writable
    assert_eq!(pool.alloc_writable_media(start_time + 3)?, tape1_uuid);

    // mark tape1 a Full
    pool.set_media_status_full(&tape1_uuid)?;

    // next call fail because there is no free media
    assert!(pool.alloc_writable_media(start_time + 5).is_err());

    // Create new media set, so that previous set can expire
    pool.start_write_session(start_time + 10, false)?;

    assert!(pool.alloc_writable_media(start_time + 10).is_err());
    assert!(pool.alloc_writable_media(start_time + 11).is_err());

    // tape1 is now expired
    assert_eq!(pool.alloc_writable_media(start_time + 12)?, tape1_uuid);

    Ok(())
}
