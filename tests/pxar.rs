use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

// Test if xattrs are correctly archived and restored
#[test]
fn pxar_create_and_extract() {
    let src_dir = "./tests/catar_data/test_xattrs_src/";
    let dest_dir = "./tests/catar_data/test_xattrs_dest/";

    let exec_path = if cfg!(debug_assertions) {
        "./target/debug/pxar"
    } else {
        "./target/release/pxar"
    };

    println!("run '{} create archive.pxar {}'", exec_path, src_dir);

    Command::new(exec_path)
        .arg("create")
        .arg("./tests/archive.pxar")
        .arg(src_dir)
        .status()
        .unwrap_or_else(|err| {
            panic!("Failed to invoke '{}': {}", exec_path, err)
        });

    Command::new(exec_path)
        .arg("extract")
        .arg("./tests/archive.pxar")
        .arg(dest_dir)
        .status()
        .unwrap_or_else(|err| {
            panic!("Failed to invoke '{}': {}", exec_path, err)
        });

    /* Use rsync with --dry-run and --itemize-changes to compare
       src_dir and dest_dir */
    let stdout = Command::new("rsync")
        .arg("--dry-run")
        .arg("--itemize-changes")
        .arg("--recursive")
        .arg("--acls")
        .arg("--xattrs")
        .arg("--owner")
        .arg("--group")
        .arg("--hard-links")
        .arg(src_dir)
        .arg(dest_dir)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap()
        .stdout
        .unwrap();

    let reader = BufReader::new(stdout);
    let linecount = reader.lines().fold(0, |count, _| count + 1 );
    println!("Rsync listed {} differences to address", linecount);

    // Cleanup archive
    Command::new("rm")
        .arg("./tests/archive.pxar")
        .status()
        .unwrap_or_else(|err| {
            panic!("Failed to invoke 'rm': {}", err)
        });

    // Cleanup destination dir
    Command::new("rm")
        .arg("-r")
        .arg(dest_dir)
        .status()
        .unwrap_or_else(|err| {
            panic!("Failed to invoke 'rm': {}", err)
        });

    // If source and destination folder contain the same content,
    // the output of the rsync invokation should yield no lines.
    if linecount != 0 {
        panic!("pxar create and extract did not yield the same contents");
    }
}
