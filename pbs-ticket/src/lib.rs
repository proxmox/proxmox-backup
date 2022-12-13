//! Generate and verify Authentication tickets

use std::borrow::Cow;
use std::io;
use std::marker::PhantomData;

use anyhow::{bail, format_err, Error};
use openssl::hash::MessageDigest;
use openssl::pkey::{HasPublic, PKey, Private};
use openssl::sign::{Signer, Verifier};
use percent_encoding::{percent_decode_str, percent_encode, AsciiSet};

pub const TICKET_LIFETIME: i64 = 3600 * 2; // 2 hours

pub const TERM_PREFIX: &str = "PBSTERM";

/// Stringified ticket data must not contain colons...
const TICKET_ASCIISET: &AsciiSet = &percent_encoding::CONTROLS.add(b':');

/// An empty type implementing [`ToString`] and [`FromStr`](std::str::FromStr), used for tickets
/// with no data.
pub struct Empty;

impl ToString for Empty {
    fn to_string(&self) -> String {
        String::new()
    }
}

impl std::str::FromStr for Empty {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        if !s.is_empty() {
            bail!("unexpected ticket data, should be empty");
        }
        Ok(Empty)
    }
}

/// An API ticket consists of a ticket type (prefix), type-dependent data, optional additional
/// authenticaztion data, a timestamp and a signature. We store these values in the form
/// `<prefix>:<stringified data>:<timestamp>::<signature>`.
///
/// The signature is made over the string consisting of prefix, data, timestamp and aad joined
/// together by colons. If there is no additional authentication data it will be skipped together
/// with the colon separating it from the timestamp.
pub struct Ticket<T>
where
    T: ToString + std::str::FromStr,
{
    prefix: Cow<'static, str>,
    data: String,
    time: i64,
    signature: Option<Vec<u8>>,
    _type_marker: PhantomData<fn() -> T>,
}

impl<T> Ticket<T>
where
    T: ToString + std::str::FromStr,
    <T as std::str::FromStr>::Err: std::fmt::Debug,
{
    /// Prepare a new ticket for signing.
    pub fn new(prefix: &'static str, data: &T) -> Result<Self, Error> {
        Ok(Self {
            prefix: Cow::Borrowed(prefix),
            data: data.to_string(),
            time: proxmox_time::epoch_i64(),
            signature: None,
            _type_marker: PhantomData,
        })
    }

    /// Get the ticket prefix.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Get the ticket's time stamp in seconds since the unix epoch.
    pub fn time(&self) -> i64 {
        self.time
    }

    /// Get the raw string data contained in the ticket. The `verify` method will call `parse()`
    /// this in the end, so using this method directly is discouraged as it does not verify the
    /// signature.
    pub fn raw_data(&self) -> &str {
        &self.data
    }

    /// Serialize the ticket into a writer.
    ///
    /// This only writes a string. We use `io::write` instead of `fmt::Write` so we can reuse the
    /// same function for openssl's `Verify`, which only implements `io::Write`.
    fn write_data(&self, f: &mut dyn io::Write) -> Result<(), Error> {
        write!(
            f,
            "{}:{}:{:08X}",
            percent_encode(self.prefix.as_bytes(), TICKET_ASCIISET),
            percent_encode(self.data.as_bytes(), TICKET_ASCIISET),
            self.time,
        )
        .map_err(Error::from)
    }

    /// Write additional authentication data to the verifier.
    fn write_aad(f: &mut dyn io::Write, aad: Option<&str>) -> Result<(), Error> {
        if let Some(aad) = aad {
            write!(f, ":{}", percent_encode(aad.as_bytes(), TICKET_ASCIISET))?;
        }
        Ok(())
    }

    /// Change the ticket's time, used mostly for testing.
    #[cfg(test)]
    fn change_time(&mut self, time: i64) -> &mut Self {
        self.time = time;
        self
    }

    /// Sign the ticket.
    pub fn sign(&mut self, keypair: &PKey<Private>, aad: Option<&str>) -> Result<String, Error> {
        let mut output = Vec::<u8>::new();
        let mut signer = Signer::new(MessageDigest::sha256(), keypair)
            .map_err(|err| format_err!("openssl error creating signer for ticket: {}", err))?;

        self.write_data(&mut output)
            .map_err(|err| format_err!("error creating ticket: {}", err))?;

        signer
            .update(&output)
            .map_err(Error::from)
            .and_then(|()| Self::write_aad(&mut signer, aad))
            .map_err(|err| format_err!("error signing ticket: {}", err))?;

        // See `Self::write_data` for why this is safe
        let mut output = unsafe { String::from_utf8_unchecked(output) };

        let signature = signer
            .sign_to_vec()
            .map_err(|err| format_err!("error finishing ticket signature: {}", err))?;

        use std::fmt::Write;
        write!(
            &mut output,
            "::{}",
            base64::encode_config(&signature, base64::STANDARD_NO_PAD),
        )?;

        self.signature = Some(signature);

        Ok(output)
    }

    /// `verify` with an additional time frame parameter, not usually required since we always use
    /// the same time frame.
    pub fn verify_with_time_frame<P: HasPublic>(
        &self,
        keypair: &PKey<P>,
        prefix: &str,
        aad: Option<&str>,
        time_frame: std::ops::Range<i64>,
    ) -> Result<T, Error> {
        if self.prefix != prefix {
            bail!("ticket with invalid prefix");
        }

        let signature = match self.signature.as_ref() {
            Some(sig) => sig,
            None => bail!("invalid ticket without signature"),
        };

        let age = proxmox_time::epoch_i64() - self.time;
        if age < time_frame.start {
            bail!("invalid ticket - timestamp newer than expected");
        }
        if age > time_frame.end {
            bail!("invalid ticket - expired");
        }

        let mut verifier = Verifier::new(MessageDigest::sha256(), keypair)?;

        self.write_data(&mut verifier)
            .and_then(|()| Self::write_aad(&mut verifier, aad))
            .map_err(|err| format_err!("error verifying ticket: {}", err))?;

        let is_valid: bool = verifier
            .verify(signature)
            .map_err(|err| format_err!("openssl error verifying ticket: {}", err))?;

        if !is_valid {
            bail!("ticket with invalid signature");
        }

        self.data
            .parse()
            .map_err(|err| format_err!("failed to parse contained ticket data: {:?}", err))
    }

    /// Verify the ticket with the provided key pair. The additional authentication data needs to
    /// match the one used when generating the ticket, and the ticket's age must fall into the time
    /// frame.
    pub fn verify<P: HasPublic>(
        &self,
        keypair: &PKey<P>,
        prefix: &str,
        aad: Option<&str>,
    ) -> Result<T, Error> {
        self.verify_with_time_frame(keypair, prefix, aad, -300..TICKET_LIFETIME)
    }

    /// Parse a ticket string.
    pub fn parse(ticket: &str) -> Result<Self, Error> {
        let mut parts = ticket.splitn(4, ':');

        let prefix = percent_decode_str(
            parts
                .next()
                .ok_or_else(|| format_err!("ticket without prefix"))?,
        )
        .decode_utf8()
        .map_err(|err| format_err!("invalid ticket, error decoding prefix: {}", err))?;

        let data = percent_decode_str(
            parts
                .next()
                .ok_or_else(|| format_err!("ticket without data"))?,
        )
        .decode_utf8()
        .map_err(|err| format_err!("invalid ticket, error decoding data: {}", err))?;

        let time = i64::from_str_radix(
            parts
                .next()
                .ok_or_else(|| format_err!("ticket without timestamp"))?,
            16,
        )
        .map_err(|err| format_err!("ticket with bad timestamp: {}", err))?;

        let remainder = parts
            .next()
            .ok_or_else(|| format_err!("ticket without signature"))?;
        // <prefix>:<data>:<time>::signature - the 4th `.next()` swallows the first colon in the
        // double-colon!
        if !remainder.starts_with(':') {
            bail!("ticket without signature separator");
        }
        let signature = base64::decode_config(&remainder[1..], base64::STANDARD_NO_PAD)
            .map_err(|err| format_err!("ticket with bad signature: {}", err))?;

        Ok(Self {
            prefix: Cow::Owned(prefix.into_owned()),
            data: data.into_owned(),
            time,
            signature: Some(signature),
            _type_marker: PhantomData,
        })
    }
}

#[cfg(test)]
mod test {
    use std::convert::Infallible;
    use std::fmt;

    use openssl::pkey::{PKey, Private};

    use super::Ticket;

    #[derive(Debug, Eq, PartialEq)]
    struct Testid(String);

    impl std::str::FromStr for Testid {
        type Err = Infallible;

        fn from_str(s: &str) -> Result<Self, Infallible> {
            Ok(Self(s.to_string()))
        }
    }

    impl fmt::Display for Testid {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    fn simple_test<F>(key: &PKey<Private>, aad: Option<&str>, modify: F)
    where
        F: FnOnce(&mut Ticket<Testid>) -> bool,
    {
        let userid = Testid("root".to_string());

        let mut ticket = Ticket::new("PREFIX", &userid).expect("failed to create Ticket struct");
        let should_work = modify(&mut ticket);
        let ticket = ticket.sign(key, aad).expect("failed to sign test ticket");

        let parsed =
            Ticket::<Testid>::parse(&ticket).expect("failed to parse generated test ticket");
        if should_work {
            let check: Testid = parsed
                .verify(key, "PREFIX", aad)
                .expect("failed to verify test ticket");

            assert_eq!(userid, check);
        } else {
            parsed
                .verify(key, "PREFIX", aad)
                .expect_err("failed to verify test ticket");
        }
    }

    #[test]
    fn test_tickets() {
        // first we need keys, for testing we use small keys for speed...
        let rsa =
            openssl::rsa::Rsa::generate(1024).expect("failed to generate RSA key for testing");
        let key = openssl::pkey::PKey::<openssl::pkey::Private>::from_rsa(rsa)
            .expect("failed to create PKey for RSA key");

        simple_test(&key, Some("secret aad data"), |_| true);
        simple_test(&key, None, |_| true);
        simple_test(&key, None, |t| {
            t.change_time(0);
            false
        });
        simple_test(&key, None, |t| {
            t.change_time(proxmox_time::epoch_i64() + 0x1000_0000);
            false
        });
    }
}
