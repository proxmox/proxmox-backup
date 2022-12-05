//! Deals with the server's current certificates (proxy.pem).

use std::mem::MaybeUninit;
use std::path::PathBuf;

use anyhow::{bail, format_err, Error};
use foreign_types::ForeignTypeRef;
use openssl::pkey::{PKey, Public};
use openssl::stack::Stack;
use openssl::x509::{GeneralName, X509};

// C type:
#[allow(non_camel_case_types)]
type ASN1_TIME = <openssl::asn1::Asn1TimeRef as ForeignTypeRef>::CType;

extern "C" {
    fn ASN1_TIME_to_tm(s: *const ASN1_TIME, tm: *mut libc::tm) -> libc::c_int;
}

fn asn1_time_to_unix(time: &openssl::asn1::Asn1TimeRef) -> Result<i64, Error> {
    let mut c_tm = MaybeUninit::<libc::tm>::uninit();
    let rc = unsafe { ASN1_TIME_to_tm(time.as_ptr(), c_tm.as_mut_ptr()) };
    if rc != 1 {
        bail!("failed to parse ASN1 time");
    }
    let mut c_tm = unsafe { c_tm.assume_init() };
    proxmox_time::timegm(&mut c_tm)
}

pub struct CertInfo {
    x509: X509,
}

fn x509name_to_string(name: &openssl::x509::X509NameRef) -> Result<String, Error> {
    let mut parts = Vec::new();
    for entry in name.entries() {
        parts.push(format!(
            "{} = {}",
            entry.object().nid().short_name()?,
            entry.data().as_utf8()?
        ));
    }
    Ok(parts.join(", "))
}

impl CertInfo {
    pub fn from_path(path: PathBuf) -> Result<Self, Error> {
        Self::from_pem(&proxmox_sys::fs::file_get_contents(&path)?)
            .map_err(|err| format_err!("failed to load certificate from {:?} - {}", path, err))
    }

    pub fn from_pem(cert_pem: &[u8]) -> Result<Self, Error> {
        let x509 = openssl::x509::X509::from_pem(cert_pem)?;
        Ok(Self { x509 })
    }

    pub fn subject_alt_names(&self) -> Option<Stack<GeneralName>> {
        self.x509.subject_alt_names()
    }

    pub fn subject_name(&self) -> Result<String, Error> {
        x509name_to_string(self.x509.subject_name())
    }

    pub fn issuer_name(&self) -> Result<String, Error> {
        x509name_to_string(self.x509.issuer_name())
    }

    pub fn fingerprint(&self) -> Result<String, Error> {
        let fp = self.x509.digest(openssl::hash::MessageDigest::sha256())?;
        Ok(hex::encode(fp)
            .as_bytes()
            .chunks(2)
            .map(|v| std::str::from_utf8(v).unwrap())
            .collect::<Vec<&str>>()
            .join(":"))
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

    pub fn not_before_unix(&self) -> Result<i64, Error> {
        asn1_time_to_unix(self.not_before())
    }

    pub fn not_after_unix(&self) -> Result<i64, Error> {
        asn1_time_to_unix(self.not_after())
    }

    /// Check if the certificate is expired at or after a specific unix epoch.
    pub fn is_expired_after_epoch(&self, epoch: i64) -> Result<bool, Error> {
        Ok(self.not_after_unix()? < epoch)
    }
}
