use std::path::Path;
use std::process::Command;

use lazy_static::lazy_static;

use crate::config::datastore;
use crate::tools::subscription::read_subscription;

lazy_static! {
    static ref FILES: Vec<&'static str> = vec!["/etc/hosts", "/etc/network/interfaces"];

    // (<command>, <arg [, arg]>)
    static ref COMMANDS: Vec<(&'static str, Vec<&'static str>)> = vec![
        ("/usr/bin/df", vec!["-h"]),
        ("/usr/bin/lsblk", vec!["-ascii"])
    ];

    // (<description>, <function to call>)
    static ref FUNCTION_CALLS: Vec<(&'static str, fn() -> String)> = vec![
        ("Subscription status", || match read_subscription() {
            Ok(Some(sub_info)) => sub_info.status.to_string(),
            _ => String::from("No subscription found"),
        }),
        ("Datastores", || {
            let config = match datastore::config() {
                Ok((config, _digest)) => config,
                _ => return String::from("could not read datastore config"),
            };

            let mut list = Vec::new();
            for (store, _) in &config.sections {
                list.push(store.as_str());
            }
            list.join(", ")
        })
    ];
}

pub fn generate_report() -> String {
    use proxmox::tools::fs::file_read_optional_string;

    let file_contents = FILES
        .iter()
        .map(|file_name| {
            let content = match file_read_optional_string(Path::new(file_name)) {
                Ok(Some(content)) => content,
                Err(err) => err.to_string(),
                _ => String::from("Could not be read!"),
            };
            format!("# {}\n{}", file_name, content)
        })
        .collect::<Vec<String>>()
        .join("\n\n");

    let command_outputs = COMMANDS
        .iter()
        .map(|(command, args)| {
            let output = match Command::new(command).args(args).output() {
                Ok(output) => String::from_utf8_lossy(&output.stdout).to_string(),
                Err(err) => err.to_string(),
            };
            format!("# {} {}\n{}", command, args.join(" "), output)
        })
        .collect::<Vec<String>>()
        .join("\n\n");

    let function_outputs = FUNCTION_CALLS
        .iter()
        .map(|(desc, function)| format!("# {}\n{}", desc, function()))
        .collect::<Vec<String>>()
        .join("\n\n");

    format!(
        " FILES\n{}\n COMMANDS\n{}\n FUNCTIONS\n{}",
        file_contents, command_outputs, function_outputs
    )
}
