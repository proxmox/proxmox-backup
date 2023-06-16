//! Proxmox Backup Server Configuration library
//!
//! This library contains helper to read, parse and write the
//! configuration files.

use anyhow::{bail, format_err, Error};
use nix::sys::stat::Mode;
use openssl::pkey::PKey;
use openssl::rsa::Rsa;
use openssl::x509::X509Builder;
use std::path::PathBuf;

use proxmox_lang::try_block;

use pbs_buildcfg::{self, configdir};

pub mod acme;
pub mod node;
pub mod tfa;

/// Check configuration directory permissions
///
/// For security reasons, we want to make sure they are set correctly:
/// * owned by 'backup' user/group
/// * nobody else can read (mode 0700)
pub fn check_configdir_permissions() -> Result<(), Error> {
    let cfgdir = pbs_buildcfg::CONFIGDIR;

    let backup_user = pbs_config::backup_user()?;
    let backup_uid = backup_user.uid.as_raw();
    let backup_gid = backup_user.gid.as_raw();

    try_block!({
        let stat = nix::sys::stat::stat(cfgdir)?;

        if stat.st_uid != backup_uid {
            bail!("wrong user ({} != {})", stat.st_uid, backup_uid);
        }
        if stat.st_gid != backup_gid {
            bail!("wrong group ({} != {})", stat.st_gid, backup_gid);
        }

        let perm = stat.st_mode & 0o777;
        if perm != 0o700 {
            bail!("wrong permission ({:o} != {:o})", perm, 0o700);
        }
        Ok(())
    })
    .map_err(|err| {
        format_err!(
            "configuration directory '{}' permission problem - {}",
            cfgdir,
            err
        )
    })
}

pub fn create_configdir() -> Result<(), Error> {
    let cfgdir = pbs_buildcfg::CONFIGDIR;

    match nix::unistd::mkdir(cfgdir, Mode::from_bits_truncate(0o700)) {
        Ok(()) => {}
        Err(nix::errno::Errno::EEXIST) => {
            check_configdir_permissions()?;
            return Ok(());
        }
        Err(err) => bail!(
            "unable to create configuration directory '{}' - {}",
            cfgdir,
            err
        ),
    }

    let backup_user = pbs_config::backup_user()?;

    nix::unistd::chown(cfgdir, Some(backup_user.uid), Some(backup_user.gid)).map_err(|err| {
        format_err!(
            "unable to set configuration directory '{}' permissions - {}",
            cfgdir,
            err
        )
    })
}

/// Update self signed node certificate.
pub fn update_self_signed_cert(force: bool) -> Result<(), Error> {
    let key_path = PathBuf::from(configdir!("/proxy.key"));
    let cert_path = PathBuf::from(configdir!("/proxy.pem"));

    if key_path.exists() && cert_path.exists() && !force {
        return Ok(());
    }

    let rsa = Rsa::generate(4096).unwrap();

    let priv_pem = rsa.private_key_to_pem()?;

    let mut x509 = X509Builder::new()?;

    x509.set_version(2)?;

    let today = openssl::asn1::Asn1Time::days_from_now(0)?;
    x509.set_not_before(&today)?;
    let expire = openssl::asn1::Asn1Time::days_from_now(365 * 1000)?;
    x509.set_not_after(&expire)?;

    let nodename = proxmox_sys::nodename();
    let mut fqdn = nodename.to_owned();

    let resolv_conf = crate::api2::node::dns::read_etc_resolv_conf()?;
    if let Some(search) = resolv_conf["search"].as_str() {
        fqdn.push('.');
        fqdn.push_str(search);
    }

    // we try to generate an unique 'subject' to avoid browser problems
    //(reused serial numbers, ..)
    let uuid = proxmox_uuid::Uuid::generate();

    let mut subject_name = openssl::x509::X509NameBuilder::new()?;
    subject_name.append_entry_by_text("O", "Proxmox Backup Server")?;
    subject_name.append_entry_by_text("OU", &format!("{:X}", uuid))?;
    subject_name.append_entry_by_text("CN", &fqdn)?;
    let subject_name = subject_name.build();

    x509.set_subject_name(&subject_name)?;
    x509.set_issuer_name(&subject_name)?;

    let bc = openssl::x509::extension::BasicConstraints::new(); // CA = false
    let bc = bc.build()?;
    x509.append_extension(bc)?;

    let usage = openssl::x509::extension::ExtendedKeyUsage::new()
        .server_auth()
        .build()?;
    x509.append_extension(usage)?;

    let context = x509.x509v3_context(None, None);

    let mut alt_names = openssl::x509::extension::SubjectAlternativeName::new();

    alt_names.ip("127.0.0.1");
    alt_names.ip("::1");

    alt_names.dns("localhost");

    if nodename != "localhost" {
        alt_names.dns(nodename);
    }
    if nodename != fqdn {
        alt_names.dns(&fqdn);
    }

    let alt_names = alt_names.build(&context)?;

    x509.append_extension(alt_names)?;

    let pub_pem = rsa.public_key_to_pem()?;
    let pubkey = PKey::public_key_from_pem(&pub_pem)?;

    x509.set_pubkey(&pubkey)?;

    let context = x509.x509v3_context(None, None);
    let ext = openssl::x509::extension::SubjectKeyIdentifier::new().build(&context)?;
    x509.append_extension(ext)?;

    let context = x509.x509v3_context(None, None);
    let ext = openssl::x509::extension::AuthorityKeyIdentifier::new()
        .keyid(true)
        .build(&context)?;
    x509.append_extension(ext)?;

    let privkey = PKey::from_rsa(rsa)?;

    x509.sign(&privkey, openssl::hash::MessageDigest::sha256())?;

    let x509 = x509.build();
    let cert_pem = x509.to_pem()?;

    set_proxy_certificate(&cert_pem, &priv_pem)?;

    Ok(())
}

pub(crate) fn set_proxy_certificate(cert_pem: &[u8], key_pem: &[u8]) -> Result<(), Error> {
    let key_path = PathBuf::from(configdir!("/proxy.key"));
    let cert_path = PathBuf::from(configdir!("/proxy.pem"));

    create_configdir()?;
    pbs_config::replace_backup_config(key_path, key_pem)
        .map_err(|err| format_err!("error writing certificate private key - {}", err))?;
    pbs_config::replace_backup_config(cert_path, cert_pem)
        .map_err(|err| format_err!("error writing certificate file - {}", err))?;

    Ok(())
}
