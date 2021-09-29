use anyhow::Error;

use crate::auth_helpers::private_auth_key;

/// As root we have access to the private key file and can use it directly. Otherwise the connect
/// call will interactively query the password.
pub fn connect_to_localhost() -> Result<pbs_client::HttpClient, Error> {
    pbs_client::connect_to_localhost(if nix::unistd::Uid::current().is_root() {
        Some(private_auth_key())
    } else {
        None
    })
}
