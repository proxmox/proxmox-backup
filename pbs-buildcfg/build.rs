// build.rs
use std::env;
use std::process::Command;

fn main() {
    let repoid = match env::var("REPOID") {
        Ok(repoid) => repoid,
        Err(_) => match Command::new("git").args(["rev-parse", "HEAD"]).output() {
            Ok(output) => String::from_utf8(output.stdout).unwrap(),
            Err(err) => {
                panic!("git rev-parse failed: {}", err);
            }
        },
    };

    println!("cargo:rustc-env=REPOID={}", repoid);
}
