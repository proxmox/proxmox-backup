//! Generate and verify Authentication tickets

use anyhow::{bail, Error};
use base64;

use openssl::pkey::{PKey, Public, Private};
use openssl::sign::{Signer, Verifier};
use openssl::hash::MessageDigest;

use crate::api2::types::Userid;
use crate::tools::epoch_now_u64;

pub const TICKET_LIFETIME: i64 = 3600*2; // 2 hours

const TERM_PREFIX: &str = "PBSTERM";

pub fn assemble_term_ticket(
    keypair: &PKey<Private>,
    userid: &Userid,
    path: &str,
    port: u16,
) -> Result<String, Error> {
    assemble_rsa_ticket(
        keypair,
        TERM_PREFIX,
        None,
        Some(&format!("{}{}{}", userid, path, port)),
    )
}

pub fn verify_term_ticket(
    keypair: &PKey<Public>,
    userid: &Userid,
    path: &str,
    port: u16,
    ticket: &str,
) -> Result<(i64, Option<Userid>), Error> {
    verify_rsa_ticket(
        keypair,
        TERM_PREFIX,
        ticket,
        Some(&format!("{}{}{}", userid, path, port)),
        -300,
        TICKET_LIFETIME,
    )
}

pub fn assemble_rsa_ticket(
    keypair: &PKey<Private>,
    prefix: &str,
    data: Option<&Userid>,
    secret_data: Option<&str>,
) -> Result<String, Error> {

    let epoch = epoch_now_u64()?;

    let timestamp = format!("{:08X}", epoch);

    let mut plain = prefix.to_owned();
    plain.push(':');

    if let Some(data) = data {
        use std::fmt::Write;
        write!(plain, "{}", data)?;
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
) -> Result<(i64, Option<Userid>), Error> {

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
    let now = epoch_now_u64()? as i64;

    let age = now - timestamp;
    if age < min_age {
        bail!("invalid ticket - timestamp newer than expected.");
    }

    if age > max_age {
        bail!("invalid ticket - timestamp too old.");
    }

    Ok((age, data.map(|s| s.parse()).transpose()?))
}
