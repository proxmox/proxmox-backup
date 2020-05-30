use std::path::PathBuf;

use anyhow::{bail, Error};

use proxmox::api::{api, cli::*};

use proxmox_backup::config;
use proxmox_backup::configdir;
use proxmox_backup::auth_helpers::*;

fn x509name_to_string(name: &openssl::x509::X509NameRef) -> Result<String, Error> {
    let mut parts = Vec::new();
    for entry in name.entries() {
        parts.push(format!("{} = {}", entry.object().nid().short_name()?, entry.data().as_utf8()?));
    }
    Ok(parts.join(", "))
}

#[api]
/// Display node certificate information.
fn cert_info() -> Result<(), Error> {

    let cert_path = PathBuf::from(configdir!("/proxy.pem"));

    let cert_pem = proxmox::tools::fs::file_get_contents(&cert_path)?;

    let cert = openssl::x509::X509::from_pem(&cert_pem)?;

    println!("Subject: {}", x509name_to_string(cert.subject_name())?);

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

    println!("Issuer: {}", x509name_to_string(cert.issuer_name())?);
    println!("Validity:");
    println!("    Not Before: {}", cert.not_before());
    println!("    Not After : {}", cert.not_after());

    let fp = cert.digest(openssl::hash::MessageDigest::sha256())?;
    let fp_string = proxmox::tools::digest_to_hex(&fp);
    let fp_string = fp_string.as_bytes().chunks(2).map(|v| std::str::from_utf8(v).unwrap())
        .collect::<Vec<&str>>().join(":");

    println!("Fingerprint (sha256): {}", fp_string);

    let pubkey = cert.public_key()?;
    println!("Public key type: {}", openssl::nid::Nid::from_raw(pubkey.id().as_raw()).long_name()?);
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
