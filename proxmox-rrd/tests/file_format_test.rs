use std::path::Path;
use std::process::Command;

use anyhow::{bail, Error};

use proxmox_rrd::rrd::RRD;
use proxmox_sys::fs::CreateOptions;

fn compare_file(fn1: &str, fn2: &str) -> Result<(), Error> {
    let status = Command::new("/usr/bin/cmp")
        .arg(fn1)
        .arg(fn2)
        .status()
        .expect("failed to execute process");

    if !status.success() {
        bail!("file compare failed");
    }

    Ok(())
}

const RRD_V1_FN: &str = "./tests/testdata/cpu.rrd_v1";
const RRD_V2_FN: &str = "./tests/testdata/cpu.rrd_v2";

// make sure we can load and convert RRD v1
#[test]
fn upgrade_from_rrd_v1() -> Result<(), Error> {
    let rrd = RRD::load(Path::new(RRD_V1_FN), true)?;

    const RRD_V2_NEW_FN: &str = "./tests/testdata/cpu.rrd_v2.upgraded";
    let new_path = Path::new(RRD_V2_NEW_FN);
    rrd.save(new_path, CreateOptions::new(), true)?;

    let result = compare_file(RRD_V2_FN, RRD_V2_NEW_FN);
    let _ = std::fs::remove_file(RRD_V2_NEW_FN);
    result?;

    Ok(())
}

// make sure we can load and save RRD v2
#[test]
fn load_and_save_rrd_v2() -> Result<(), Error> {
    let rrd = RRD::load(Path::new(RRD_V2_FN), true)?;

    const RRD_V2_NEW_FN: &str = "./tests/testdata/cpu.rrd_v2.saved";
    let new_path = Path::new(RRD_V2_NEW_FN);
    rrd.save(new_path, CreateOptions::new(), true)?;

    let result = compare_file(RRD_V2_FN, RRD_V2_NEW_FN);
    let _ = std::fs::remove_file(RRD_V2_NEW_FN);
    result?;

    Ok(())
}
