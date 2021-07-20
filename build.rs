// build.rs
use std::env;
use std::process::Command;

fn git_command(args: &[&str]) -> String {
    match Command::new("git").args(args).output() {
        Ok(output) => String::from_utf8(output.stdout).unwrap().trim_end().to_string(),
        Err(err) => {
            panic!("git {:?} failed: {}", args, err);
        }
    }
}

fn main() {
    let repoid = match env::var("REPOID") {
        Ok(repoid) => repoid,
        Err(_) => git_command(&["rev-parse", "HEAD"]),
    };

    println!("cargo:rustc-env=REPOID={}", repoid);
}
