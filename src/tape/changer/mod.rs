mod email;
pub use email::*;

mod parse_mtx_status;
pub use parse_mtx_status::*;

mod mtx_wrapper;
pub use mtx_wrapper::*;

mod linux_tape;
pub use linux_tape::*;

use anyhow::Error;

/// Interface to media change devices
pub trait MediaChange {

    /// Load media into drive
    ///
    /// This unloads first if the drive is already loaded with another media.
    fn load_media(&mut self, changer_id: &str) -> Result<(), Error>;

    /// Unload media from drive
    ///
    /// This is a nop on drives without autoloader.
    fn unload_media(&mut self) -> Result<(), Error>;

    /// Returns true if unload_media automatically ejects drive media
    fn eject_on_unload(&self) -> bool {
        false
    }

    /// List media changer IDs (barcodes)
    fn list_media_changer_ids(&self) -> Result<Vec<String>, Error>;
}
