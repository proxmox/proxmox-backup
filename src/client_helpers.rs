use anyhow::Error;

use pbs_api_types::{Authid, Userid};
use pbs_client::{HttpClient, HttpClientOptions};

use proxmox_auth_api::ticket::Ticket;

use crate::auth::private_auth_keyring;

/// Connect to localhost:8007 as root@pam
///
/// This automatically creates a ticket if run as 'root' user.
pub fn connect_to_localhost() -> Result<pbs_client::HttpClient, Error> {
    let options = if nix::unistd::Uid::current().is_root() {
        let ticket =
            Ticket::new("PBS", Userid::root_userid())?.sign(private_auth_keyring(), None)?;
        let fingerprint = crate::cert_info()?.fingerprint()?;
        HttpClientOptions::new_non_interactive(ticket, Some(fingerprint))
    } else {
        HttpClientOptions::new_interactive(None, None)
    };

    HttpClient::new("localhost", 8007, Authid::root_auth_id(), options)
}
