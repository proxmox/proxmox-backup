use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, format_err, Error};

use proxmox::sys::error::SysError;
use proxmox::tools::fs::CreateOptions;

use crate::api2::types::{
    PROXMOX_SAFE_ID_REGEX,
    KnownAcmeDirectory,
    AcmeAccountName,
};
use crate::tools::ControlFlow;

pub(crate) const ACME_DIR: &str = configdir!("/acme");
pub(crate) const ACME_ACCOUNT_DIR: &str = configdir!("/acme/accounts");

pub mod plugin;

// `const fn`ify this once it is supported in `proxmox`
fn root_only() -> CreateOptions {
    CreateOptions::new()
        .owner(nix::unistd::ROOT)
        .group(nix::unistd::Gid::from_raw(0))
        .perm(nix::sys::stat::Mode::from_bits_truncate(0o700))
}

fn create_acme_subdir(dir: &str) -> nix::Result<()> {
    match proxmox::tools::fs::create_dir(dir, root_only()) {
        Ok(()) => Ok(()),
        Err(err) if err.already_exists() => Ok(()),
        Err(err) => Err(err),
    }
}

pub(crate) fn make_acme_dir() -> nix::Result<()> {
    create_acme_subdir(ACME_DIR)
}

pub(crate) fn make_acme_account_dir() -> nix::Result<()> {
    make_acme_dir()?;
    create_acme_subdir(ACME_ACCOUNT_DIR)
}

pub const KNOWN_ACME_DIRECTORIES: &[KnownAcmeDirectory] = &[
    KnownAcmeDirectory {
        name: "Let's Encrypt V2",
        url: "https://acme-v02.api.letsencrypt.org/directory",
    },
    KnownAcmeDirectory {
        name: "Let's Encrypt V2 Staging",
        url: "https://acme-staging-v02.api.letsencrypt.org/directory",
    },
];

pub const DEFAULT_ACME_DIRECTORY_ENTRY: &KnownAcmeDirectory = &KNOWN_ACME_DIRECTORIES[0];

pub fn account_path(name: &str) -> String {
    format!("{}/{}", ACME_ACCOUNT_DIR, name)
}


pub fn foreach_acme_account<F>(mut func: F) -> Result<(), Error>
where
    F: FnMut(AcmeAccountName) -> ControlFlow<Result<(), Error>>,
{
    match crate::tools::fs::scan_subdir(-1, ACME_ACCOUNT_DIR, &PROXMOX_SAFE_ID_REGEX) {
        Ok(files) => {
            for file in files {
                let file = file?;
                let file_name = unsafe { file.file_name_utf8_unchecked() };

                if file_name.starts_with('_') {
                    continue;
                }

                let account_name = match AcmeAccountName::from_string(file_name.to_owned()) {
                    Ok(account_name) => account_name,
                    Err(_) => continue,
                };

                if let ControlFlow::Break(result) = func(account_name) {
                    return result;
                }
            }
            Ok(())
        }
        Err(err) if err.not_found() => Ok(()),
        Err(err) => Err(err.into()),
    }
}

/// Run a function for each DNS plugin ID.
pub fn foreach_dns_plugin<F>(mut func: F) -> Result<(), Error>
where
    F: FnMut(&str) -> ControlFlow<Result<(), Error>>,
{
    match crate::tools::fs::read_subdir(-1, "/usr/share/proxmox-acme/dnsapi") {
        Ok(files) => {
            for file in files.filter_map(Result::ok) {
                if let Some(id) = file
                    .file_name()
                    .to_str()
                    .ok()
                    .and_then(|name| name.strip_prefix("dns_"))
                    .and_then(|name| name.strip_suffix(".sh"))
                {
                    if let ControlFlow::Break(result) = func(id) {
                        return result;
                    }
                }
            }

            Ok(())
        }
        Err(err) if err.not_found() => Ok(()),
        Err(err) => Err(err.into()),
    }
}

pub fn mark_account_deactivated(name: &str) -> Result<(), Error> {
    let from = account_path(name);
    for i in 0..100 {
        let to = account_path(&format!("_deactivated_{}_{}", name, i));
        if !Path::new(&to).exists() {
            return std::fs::rename(&from, &to).map_err(|err| {
                format_err!(
                    "failed to move account path {:?} to {:?} - {}",
                    from,
                    to,
                    err
                )
            });
        }
    }
    bail!(
        "No free slot to rename deactivated account {:?}, please cleanup {:?}",
        from,
        ACME_ACCOUNT_DIR
    );
}

pub fn complete_acme_account(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    let mut out = Vec::new();
    let _ = foreach_acme_account(|name| {
        out.push(name.into_string());
        ControlFlow::CONTINUE
    });
    out
}

pub fn complete_acme_plugin(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match plugin::config() {
        Ok((config, _digest)) => config
            .iter()
            .map(|(id, (_type, _cfg))| id.clone())
            .collect(),
        Err(_) => Vec::new(),
    }
}

pub fn complete_acme_plugin_type(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    vec!["dns".to_string(), "http".to_string()]
}
