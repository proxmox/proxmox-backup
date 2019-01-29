//! Generate and verify Authentification tickets

use crate::tools;

use failure::*;
use std::path::PathBuf;
use base64;

use openssl::rsa::{Rsa};
use openssl::pkey::{PKey, Public, Private};
use openssl::sign::{Signer, Verifier};
use openssl::hash::MessageDigest;

pub fn assemble_rsa_ticket(
    keypair: &PKey<Private>,
    prefix: &str,
    data: Option<&str>,
    secret_data: Option<&str>,
) -> Result<String, Error> {

    let epoch = std::time::SystemTime::now().duration_since(
        std::time::SystemTime::UNIX_EPOCH)?.as_secs();

    let timestamp = format!("{:08X}", epoch);

    let mut plain = prefix.to_owned();
    plain.push(':');

    if let Some(data) = data {
        plain.push_str(data);
        plain.push(':');
    }

    plain.push_str(&timestamp);

    let mut full = plain.clone();
    if let Some(secret) = secret_data {
        full.push(':');
        full.push_str(secret);
    }

    let mut signer = Signer::new(MessageDigest::sha256(), &keypair)?;
    signer.update(full.as_bytes())?;
    let sign = signer.sign_to_vec()?;

    let sign_b64 = base64::encode_config(&sign, base64::STANDARD_NO_PAD);

    Ok(format!("{}::{}", plain, sign_b64))
}

pub fn verify_rsa_ticket(
    keypair: &PKey<Public>,
    prefix: &str,
    ticket: &str,
    secret_data: Option<&str>,
    min_age: i64,
    max_age: i64,
) -> Result<(i64, Option<String>), Error> {

    use std::collections::VecDeque;

    let mut parts: VecDeque<&str> = ticket.split(':').collect();

    match parts.pop_front() {
        Some(text) => if text != prefix { bail!("ticket with invalid prefix"); }
        None => bail!("ticket without prefix"),
    }

    let sign_b64 = match parts.pop_back() {
        Some(v) => v,
        None => bail!("ticket without signature"),
    };

    match parts.pop_back() {
        Some(text) => if text != "" { bail!("ticket with invalid signature separator"); }
        None => bail!("ticket without signature separator"),
    }

    let mut data = None;

    let mut full = match parts.len() {
        2 => {
            data = Some(parts[0].to_owned());
            format!("{}:{}:{}", prefix, parts[0], parts[1])
        }
        1 => format!("{}:{}", prefix, parts[0]),
        _ => bail!("ticket with invalid number of components"),
    };

    if let Some(secret) = secret_data {
        full.push(':');
        full.push_str(secret);
    }

    let sign = base64::decode_config(sign_b64, base64::STANDARD_NO_PAD)?;

    let mut verifier = Verifier::new(MessageDigest::sha256(), &keypair)?;
    verifier.update(full.as_bytes())?;

    if !verifier.verify(&sign)? {
        bail!("ticket with invalid signature");
    }

    let timestamp = i64::from_str_radix(parts.pop_back().unwrap(), 16)?;
    let now = std::time::SystemTime::now().duration_since(
        std::time::SystemTime::UNIX_EPOCH)?.as_secs() as i64;

    let age = now - timestamp;
    if age < min_age {
        bail!("invalid ticket - timestamp newer than expected.");
    }

    if age > max_age {
        bail!("invalid ticket - timestamp too old.");
    }


    println!("TEST: {:?}", parts);
    println!("TEST1: {:?}", full);
    println!("TEST2: {} {}", timestamp, age);

    Ok((age, data))
}
