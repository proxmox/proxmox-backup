use anyhow::{Context, Error};

mod benchmark;
pub use benchmark::*;
mod mount;
pub use mount::*;
mod task;
pub use task::*;
mod catalog;
pub use catalog::*;
mod snapshot;
pub use snapshot::*;

pub mod key;

pub fn base_directories() -> Result<xdg::BaseDirectories, Error> {
    xdg::BaseDirectories::with_prefix("proxmox-backup").map_err(Error::from)
}

/// Convenience helper for better error messages:
pub fn find_xdg_file(
    file_name: impl AsRef<std::path::Path>,
    description: &'static str,
) -> Result<Option<std::path::PathBuf>, Error> {
    let file_name = file_name.as_ref();
    base_directories()
        .map(|base| base.find_config_file(file_name))
        .with_context(|| format!("error searching for {}", description))
}

pub fn place_xdg_file(
    file_name: impl AsRef<std::path::Path>,
    description: &'static str,
) -> Result<std::path::PathBuf, Error> {
    let file_name = file_name.as_ref();
    base_directories()
        .and_then(|base| {
            base.place_config_file(file_name).map_err(Error::from)
        })
        .with_context(|| format!("failed to place {} in xdg home", description))
}
