use anyhow::{bail, Error};

use proxmox_router::cli::*;
use proxmox_schema::api;

use proxmox_backup::auth_helpers::*;
use proxmox_backup::config;

#[api]
/// Display node certificate information.
fn cert_info() -> Result<(), Error> {
    let cert = proxmox_backup::cert_info()?;

    println!("Subject: {}", cert.subject_name()?);

    if let Some(san) = cert.subject_alt_names() {
        for name in san.iter() {
            if let Some(v) = name.dnsname() {
                println!("    DNS:{}", v);
            } else if let Some(v) = name.ipaddress() {
                println!("    IP:{:?}", v);
            } else if let Some(v) = name.email() {
                println!("    EMAIL:{}", v);
            } else if let Some(v) = name.uri() {
                println!("    URI:{}", v);
            }
        }
    }

    println!("Issuer: {}", cert.issuer_name()?);
    println!("Validity:");
    println!("    Not Before: {}", cert.not_before());
    println!("    Not After : {}", cert.not_after());

    println!("Fingerprint (sha256): {}", cert.fingerprint()?);

    let pubkey = cert.public_key()?;
    println!(
        "Public key type: {}",
        openssl::nid::Nid::from_raw(pubkey.id().as_raw()).long_name()?
    );
    println!("Public key bits: {}", pubkey.bits());

    Ok(())
}

#[api(
    input: {
        properties: {
            force: {
	        description: "Force generation of new SSL certifate.",
	        type:  Boolean,
	        optional:true,
	    },
        }
    },
)]
/// Update node certificates and generate all needed files/directories.
fn update_certs(force: Option<bool>) -> Result<(), Error> {
    config::create_configdir()?;

    if let Err(err) = generate_auth_key() {
        bail!("unable to generate auth key - {}", err);
    }

    if let Err(err) = generate_csrf_key() {
        bail!("unable to generate csrf key - {}", err);
    }

    config::update_self_signed_cert(force.unwrap_or(false))?;

    Ok(())
}

pub fn cert_mgmt_cli() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("info", CliCommand::new(&API_METHOD_CERT_INFO))
        .insert("update", CliCommand::new(&API_METHOD_UPDATE_CERTS));

    cmd_def.into()
}
