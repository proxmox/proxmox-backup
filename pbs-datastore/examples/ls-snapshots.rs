use std::path::PathBuf;

use anyhow::{bail, Error};

use pbs_datastore::DataStore;

fn run() -> Result<(), Error> {
    let base: PathBuf = match std::env::args().skip(1).next() {
        Some(path) => path.into(),
        None => bail!("no path passed"),
    };

    let store = unsafe { DataStore::open_path("", &base, None)? };

    for ns in store.recursive_iter_backup_ns_ok(Default::default())? {
        println!("found namespace store:/{}", ns);

        for group in store.iter_backup_groups(ns)? {
            let group = group?;
            println!("    found group {}", group);

            for snapshot in group.iter_snapshots()? {
                println!("\t{}", snapshot?);
            }
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
