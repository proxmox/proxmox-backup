//! This module provides a `Connector` used to log into a Proxmox Backup API server and connect to
//! the proxmox protocol via an HTTP Upgrade request.

use std::io::{Read, Write};
use std::net::TcpStream;

use failure::{bail, format_err, Error};
use openssl::ssl::{self, SslStream};
use url::form_urlencoded;

use crate::Client;

enum Authentication {
    Password(String),
    Ticket(String, String),
}

/// Connector used to log into a Proxmox Backup API server and open a backup protocol connection.
/// If successful, this will create a `Client` used to communicate over the Proxmox Backup
/// Protocol.
pub struct Connector {
    user: String,
    server: String,
    store: String,
    auth: Option<Authentication>,
    certificate_validation: bool,
}

fn build_login(host: &str, user: &str, pass: &str) -> Vec<u8> {
    let formdata = form_urlencoded::Serializer::new(String::new())
        .append_pair("username", user)
        .append_pair("password", pass)
        .finish();

    format!("\
        POST /api2/json/access/ticket HTTP/1.1\r\n\
        host: {}\r\n\
        content-length: {}\r\n\
        content-type: application/x-www-form-urlencoded\r\n\
        \r\n\
        {}",
        host,
        formdata.as_bytes().len(),
        formdata,
    )
    .into_bytes()
}

fn build_protocol_connect(host: &str, store: &str, ticket: &str, token: &str) -> Vec<u8> {
    format!("\
        GET /api2/json/admin/datastore/{}/test-upload HTTP/1.1\r\n\
        host: {}\r\n\
        connection: upgrade\r\n\
        upgrade: proxmox-backup-protocol-1\r\n\
        cookie: PBSAuthCookie={}\r\n\
        CSRFPreventionToken: {}\r\n\
        \r\n",
        store,
        host,
        ticket,
        token,
    )
    .into_bytes()
}

// Minimalistic http response reader. The only things we care about here are the status code and
// the payload...
fn read_http_response<T: Read>(sock: T) -> Result<(u16, Vec<u8>), Error> {
    use std::io::BufRead;
    let mut reader = std::io::BufReader::new(sock);

    let mut status = String::new();
    reader.read_line(&mut status)?;

    let status = status.trim_end();
    let mut parts = status.splitn(3, ' ');
    let _version = parts
        .next()
        .ok_or_else(|| format_err!("bad http response (missing version)"))?;
    let code = parts
        .next()
        .ok_or_else(|| format_err!("bad http response (missing status code)"))?;
    let _reason = parts.next();

    let code: u16 = code.parse()?;

    // We need the payload's length if there is one:
    let mut length: Option<u32> = None;
    let mut line = String::new();
    loop {
        line.clear();
        reader.read_line(&mut line)?;
        let line = line.trim_end();

        if line.len() == 0 {
            break;
        }

        let parts: Vec<&str> = line.splitn(2, ':').collect();
        if parts.len() != 2 {
            bail!("invalid header in http response");
        }

        let name = parts[0].trim().to_lowercase().to_string();
        let value = parts[1].trim();

        // The only important header (important to know how much we need to read!)
        if name == "content-length" {
            length = Some(value.parse()?);
        }

        // Don't care about any other header contents currently...
    }

    match length {
        None => Ok((code, Vec::new())),
        Some(length) => {
            let length = length as usize;

            let mut out = Vec::with_capacity(length);
            unsafe {
                out.set_len(length);
            }

            reader.read_exact(&mut out)?;
            Ok((code, out))
        },
    }
}

fn parse_login_response(data: &[u8]) -> Result<(String, String), Error> {
    let value: serde_json::Value = serde_json::from_slice(data)?;
    let ticket = value["data"]["ticket"]
        .as_str()
        .ok_or_else(|| format_err!("no ticket found in login response"))?
        .to_string();
    let token = value["data"]["CSRFPreventionToken"]
        .as_str()
        .ok_or_else(|| format_err!("no token found in login response"))?
        .to_string();
    Ok((ticket, token))
}

impl Connector {
    /// Create a new connector for a specified user, server and remote backup store.
    pub fn new(user: String, server: String, store: String) -> Self {
        Self {
            user,
            server,
            store,
            auth: None,
            certificate_validation: true,
        }
    }

    /// Use a password to authenticate with the remote server.
    pub fn password(mut self, pass: String) -> Self {
        self.auth = Some(Authentication::Password(pass));
        self
    }

    /// Use an already existing ticket to connect to the server.
    pub fn ticket(mut self, ticket: String, token: String) -> Self {
        self.auth = Some(Authentication::Ticket(ticket, token));
        self
    }

    /// Disable TLS certificate validation.
    pub fn certificate_validation(mut self, on: bool) -> Self {
        self.certificate_validation = on;
        self
    }

    pub(crate) fn do_connect(self) -> Result<SslStream<TcpStream>, Error> {
        if self.auth.is_none() {
            bail!("missing authentication");
        }

        let stream = TcpStream::connect(&self.server)?;

        let mut connector = ssl::SslConnector::builder(ssl::SslMethod::tls())?;
        if !self.certificate_validation {
            connector.set_verify(ssl::SslVerifyMode::NONE);
        }
        let connector = connector.build();

        let mut stream = connector.connect(&self.server, stream)?;
        let (ticket, token) = match self.auth {
            None => unreachable!(), // checked above
            Some(Authentication::Password(password)) => {
                let login_request = build_login(&self.server, &self.user, &password);
                stream.write_all(&login_request)?;

                let (code, ticket) = read_http_response(&mut stream)?;
                if code != 200 {
                    bail!("login failed");
                }

                parse_login_response(&ticket)?
            }
            Some(Authentication::Ticket(ticket, token)) => (ticket, token),
        };

        let protocol_request = build_protocol_connect(&self.server, &self.store, &ticket, &token);
        stream.write_all(&protocol_request)?;
        let (code, _empty_body) = read_http_response(&mut stream)?;
        if code != 101 {
            bail!("expected 101 Switching Protocol, received code: {}", code);
        }

        Ok(stream)
    }

    /// This creates creates a synchronous client (via blocking I/O), tries to authenticate with
    /// the server and connect to the protocol endpoint. On success, a `Client` is returned.
    pub fn connect(self) -> Result<Client<SslStream<TcpStream>>, Error> {
        let stream = self.do_connect()?;

        let mut client = Client::new(stream);
        if !client.wait_for_handshake()? {
            bail!("handshake failed");
        }
        Ok(client)
    }
}
