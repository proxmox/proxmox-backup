use std::io::Write;
use std::path::PathBuf;
use std::thread;

use anyhow::{bail, Error};

// tar handle files that shrink during backup, by simply padding with zeros.
//
// this binary run multiple thread which writes some large files, then truncates
// them in a loop.

// # tar cf test.tar ./dyntest1/
// tar: dyntest1/testfile0.dat: File shrank by 2768972800 bytes; padding with zeros
// tar: dyntest1/testfile17.dat: File shrank by 2899853312 bytes; padding with zeros
// tar: dyntest1/testfile2.dat: File shrank by 3093422080 bytes; padding with zeros
// tar: dyntest1/testfile7.dat: File shrank by 2833252864 bytes; padding with zeros

// # pxar create test.pxar ./dyntest1/
// Error: detected shrunk file "./dyntest1/testfile0.dat" (22020096 < 12679380992)

fn create_large_file(path: PathBuf) {
    println!("TEST {:?}", path);

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();

    let buffer = vec![0u8; 64 * 1024];

    loop {
        for _ in 0..64 {
            file.write_all(&buffer).unwrap();
        }
        file.sync_all().unwrap();
        //println!("TRUNCATE {:?}", path);
        file.set_len(0).unwrap();
    }
}

fn main() -> Result<(), Error> {
    let base = PathBuf::from("dyntest1");
    let _ = std::fs::create_dir(&base);

    let mut handles = vec![];
    for i in 0..20 {
        let base = base.clone();
        handles.push(thread::spawn(move || {
            create_large_file(base.join(format!("testfile{}.dat", i)));
        }));
    }

    for h in handles {
        if h.join().is_err() {
            bail!("join failed");
        }
    }

    Ok(())
}
