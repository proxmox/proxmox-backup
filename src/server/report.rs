use std::path::Path;
use std::process::Command;

fn files() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        (
            "Host & Network",
            vec!["/etc/hostname", "/etc/hosts", "/etc/network/interfaces"],
        ),
        (
            "Datastores & Remotes",
            vec!["/etc/proxmox-backup/datastore.cfg"],
        ),
        (
            "User & Access",
            vec![
                "/etc/proxmox-backup/user.cfg",
                "/etc/proxmox-backup/acl.cfg",
            ],
        ),
        ("Remotes", vec!["/etc/proxmox-backup/remote.cfg"]),
        (
            "Jobs",
            vec![
                "/etc/proxmox-backup/sync.cfg",
                "/etc/proxmox-backup/verification.cfg",
            ],
        ),
        (
            "Tape",
            vec![
                "/etc/proxmox-backup/tape.cfg",
                "/etc/proxmox-backup/media-pool.cfg",
            ],
        ),
        (
            "Others",
            vec![
                "/etc/proxmox-backup/node.cfg",
                "/etc/proxmox-backup/traffic-control.cfg",
            ],
        ),
    ]
}

fn commands() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        //  ("<command>", vec![<arg [, arg]>])
        ("date", vec!["-R"]),
        ("proxmox-backup-manager", vec!["versions", "--verbose"]),
        ("proxmox-backup-manager", vec!["subscription", "get"]),
        ("proxmox-backup-manager", vec!["ldap", "list"]),
        ("proxmox-backup-manager", vec!["openid", "list"]),
        ("df", vec!["-h"]),
        (
            "lsblk",
            vec![
                "--ascii",
                "-M",
                "-o",
                "+HOTPLUG,ROTA,PHY-SEC,FSTYPE,MODEL,TRAN",
            ],
        ),
        ("ls", vec!["-l", "/dev/disk/by-id", "/dev/disk/by-path"]),
        ("zpool", vec!["status"]),
        ("zfs", vec!["list"]),
        ("arcstat", vec![]),
    ]
}

// (description, function())
type FunctionMapping = (&'static str, fn() -> String);

fn function_calls() -> Vec<FunctionMapping> {
    vec![("Datastores", || {
        let config = match pbs_config::datastore::config() {
            Ok((config, _digest)) => config,
            _ => return String::from("could not read datastore config"),
        };

        let mut list = Vec::new();
        for store in config.sections.keys() {
            list.push(store.as_str());
        }
        list.join(", ")
    })]
}

pub fn generate_report() -> String {
    use proxmox_sys::fs::file_read_optional_string;

    let file_contents = files()
        .iter()
        .map(|group| {
            let (group, files) = group;
            let group_content = files
                .iter()
                .map(|file_name| {
                    let content = match file_read_optional_string(Path::new(file_name)) {
                        Ok(Some(content)) => content,
                        Ok(None) => String::from("# file does not exist"),
                        Err(err) => err.to_string(),
                    };
                    format!("`$ cat '{file_name}'`\n```\n{}\n```", content.trim_end())
                })
                .collect::<Vec<String>>()
                .join("\n\n");

            format!("### {group}\n\n{group_content}")
        })
        .collect::<Vec<String>>()
        .join("\n\n");

    let command_outputs = commands()
        .iter()
        .map(|(command, args)| {
            let output = Command::new(command)
                .env("PROXMOX_OUTPUT_NO_BORDER", "1")
                .args(args)
                .output();
            let output = match output {
                Ok(output) => String::from_utf8_lossy(&output.stdout).to_string(),
                Err(err) => err.to_string(),
            };
            let output = output.trim_end();
            format!("$ `{command} {}`\n```\n{output}\n```", args.join(" "))
        })
        .collect::<Vec<String>>()
        .join("\n\n");

    let function_outputs = function_calls()
        .iter()
        .map(|(desc, function)| {
            let output = function();
            format!("#### {desc}\n```\n{}\n```", output.trim_end())
        })
        .collect::<Vec<String>>()
        .join("\n\n");

    format!(
        "## FILES\n\n{file_contents}\n## COMMANDS\n\n{command_outputs}\n## FUNCTIONS\n\n{function_outputs}\n"
    )
}
