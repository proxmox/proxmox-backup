use std::path::PathBuf;

use anyhow::Error;
use openssl::x509::{X509, GeneralName};
use openssl::stack::Stack;
use openssl::pkey::{Public, PKey};

use crate::configdir;

pub struct CertInfo {
    x509: X509,
}

fn x509name_to_string(name: &openssl::x509::X509NameRef) -> Result<String, Error> {
    let mut parts = Vec::new();
    for entry in name.entries() {
        parts.push(format!("{} = {}", entry.object().nid().short_name()?, entry.data().as_utf8()?));
    }
    Ok(parts.join(", "))
}

impl CertInfo {
    pub fn new() -> Result<Self, Error> {
        Self::from_path(PathBuf::from(configdir!("/proxy.pem")))
    }

    pub fn from_path(path: PathBuf) -> Result<Self, Error> {
        let cert_pem = proxmox::tools::fs::file_get_contents(&path)?;
        let x509 = openssl::x509::X509::from_pem(&cert_pem)?;
        Ok(Self{
            x509
        })
    }

    pub fn subject_alt_names(&self) -> Option<Stack<GeneralName>> {
        self.x509.subject_alt_names()
    }

    pub fn subject_name(&self) -> Result<String, Error> {
        Ok(x509name_to_string(self.x509.subject_name())?)
    }

    pub fn issuer_name(&self) -> Result<String, Error> {
        Ok(x509name_to_string(self.x509.issuer_name())?)
    }

    pub fn fingerprint(&self) -> Result<String, Error> {
        let fp = self.x509.digest(openssl::hash::MessageDigest::sha256())?;
        let fp_string = proxmox::tools::digest_to_hex(&fp);
        let fp_string = fp_string.as_bytes().chunks(2).map(|v| std::str::from_utf8(v).unwrap())
            .collect::<Vec<&str>>().join(":");
        Ok(fp_string)
    }

    pub fn public_key(&self) -> Result<PKey<Public>, Error> {
        let pubkey = self.x509.public_key()?;
        Ok(pubkey)
    }

    pub fn not_before(&self) -> &openssl::asn1::Asn1TimeRef {
        self.x509.not_before()
    }

    pub fn not_after(&self) -> &openssl::asn1::Asn1TimeRef {
        self.x509.not_after()
    }
}
