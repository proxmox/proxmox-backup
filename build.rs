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
    let repo_path = git_command(&["rev-parse", "--show-toplevel"]);
    let repoid = match env::var("REPOID") {
        Ok(repoid) => repoid,
        Err(_) => git_command(&["rev-parse", "HEAD"]),
    };

    println!("cargo:rustc-env=REPOID={}", repoid);
    println!("cargo:rerun-if-changed={}/.git/HEAD", repo_path);
}
