use std::path::PathBuf;

use anyhow::{bail, Error};

use pbs_datastore::DataStore;

fn run() -> Result<(), Error> {
    let base: PathBuf = match std::env::args().nth(1) {
        Some(path) => path.into(),
        None => bail!("no path passed!\n\nusage: ls-snapshots <path> [<max-depth>]"),
    };
    let max_depth: Option<usize> = match std::env::args().nth(2) {
        Some(depth) => match depth.parse::<usize>() {
            Ok(depth) if depth < 8 => Some(depth),
            Ok(_) => bail!("max-depth must be < 8"),
            Err(err) => bail!("couldn't parse max-depth from {depth} - {err}"),
        },
        None => None,
    };

    let store = unsafe { DataStore::open_path("", base, None)? };

    for ns in store.recursive_iter_backup_ns_ok(Default::default(), max_depth)? {
        println!("found namespace store:/{}", ns);

        for group in store.iter_backup_groups(ns)? {
            let group = group?;
            println!("    found group {}", group.group());

            for snapshot in group.iter_snapshots()? {
                let snapshot = snapshot?;
                println!("\t{}", snapshot.dir());
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
