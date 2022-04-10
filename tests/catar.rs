use std::process::Command;

use anyhow::Error;

use pbs_client::pxar::*;

fn run_test(dir_name: &str) -> Result<(), Error> {
    println!("run pxar test {}", dir_name);

    Command::new("casync")
        .arg("make")
        .arg("test-casync.catar")
        .arg(dir_name)
        .status()
        .expect("failed to execute casync");

    let writer = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("test-proxmox.catar")?;
    let writer = pxar::encoder::sync::StandardWriter::new(writer);

    let dir = nix::dir::Dir::open(
        dir_name,
        nix::fcntl::OFlag::O_NOFOLLOW,
        nix::sys::stat::Mode::empty(),
    )?;

    let options = PxarCreateOptions {
        entries_max: ENCODER_MAX_ENTRIES,
        ..PxarCreateOptions::default()
    };

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(create_archive(
        dir,
        writer,
        Flags::DEFAULT,
        |_| Ok(()),
        None,
        options,
    ))?;

    Command::new("cmp")
        .arg("--verbose")
        .arg("test-casync.catar")
        .arg("test-proxmox.catar")
        .status()
        .expect("test failed - archives are different");

    Ok(())
}

fn run_all_tests() -> Result<(), Error> {
    run_test("tests/catar_data/test_file")?;

    run_test("tests/catar_data/test_symlink")?;

    run_test("tests/catar_data/test_subdir")?;

    run_test("tests/catar_data/test_goodbye_sort_order")?;

    run_test("tests/catar_data/test_files_and_subdirs")?;

    Ok(())
}

#[test]
#[ignore]
fn catar_simple() {
    if let Err(err) = run_all_tests() {
        eprintln!("Error: {}", err);
        std::process::exit(1);
    }
}
