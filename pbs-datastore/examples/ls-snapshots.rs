use std::path::PathBuf;

use anyhow::{bail, Error};

use pbs_datastore::{ListGroups, ListSnapshots};

fn run() -> Result<(), Error> {
    let base: PathBuf = match std::env::args().skip(1).next() {
        Some(path) => path.into(),
        None => bail!("no path passed"),
    };

    for group in ListGroups::new(base.to_owned())? {
        let group = group?;
        println!("found group {}", group);

        let group_path = base.as_path().join(group.to_string());
        for snapshot in ListSnapshots::new(group, group_path)? {
            println!("\t{}", snapshot?);
        }
    }

    Ok(())
}

fn main() {
    std::process::exit(match run() {
        Ok(_) => 0,
        Err(err) => {
            eprintln!("error: {}", err);
            1
        }
    });
}
