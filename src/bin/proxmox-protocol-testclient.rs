use std::io;
use std::process::exit;

use chrono::Utc;
use failure::*;
use futures::future::{ok, poll_fn, Future};
use futures::try_ready;
use futures::{Async, Poll};
use http::{Request, Response, StatusCode};
use hyper::rt::Stream;
use hyper::Body;
use tokio::prelude::*;
use tokio_fs::file::File;

use proxmox_protocol::Client as PmxClient;
use proxmox_protocol::{BackupStream, ChunkEntry, ChunkStream, IndexType, StreamId};

use proxmox_backup::client::BackupRepository;

// This is a temporary client using the backup protocol crate.
// Its functionality should be moved to the `proxmox-backup-client` binary instead.
// For now this is mostly here to keep in the history an alternative way of connecting to an https
// server without hyper-tls in the background.
// Note that hyper-tls just wraps native_tls, and so does tokio_tls. So the only way to get
// rid of the extra dependency would be to reimplement tokio_tls on top of the openssl crate.

type HyperConnection<T, B> = hyper::client::conn::Connection<T, B>;
type HyperConnType = HyperConnection<tokio_tls::TlsStream<tokio::net::TcpStream>, Body>;

// Create a future which connects to a TLS-enabled http server.
// This would ordinarily be covered by the Connect trait in the higher level hyper interface.
// Connect to the server, initiate TLS, finally run hyper's handshake method.
fn connect(
    domain: &str,
    port: u16,
    no_cert_validation: bool,
) -> impl Future<
    // Typing out this function signature is almost more work than copying its code body...
    Item = (hyper::client::conn::SendRequest<Body>, HyperConnType),
    Error = Error,
> {
    // tokio::net::TcpStream::connect(addr) <- this takes only a single address!
    // so we need to improvise...:
    use tokio_threadpool::blocking;

    let domain = domain.to_string();
    let domain2 = domain.clone();
    poll_fn(move || {
        blocking(|| {
            let conn =
                std::net::TcpStream::connect((domain.as_str(), port)).map_err(Error::from)?;
            tokio::net::TcpStream::from_std(conn, &Default::default()).map_err(Error::from)
        })
        .map_err(Error::from)
    })
    .map_err(Error::from)
    .flatten()
    .and_then(move |tcp| {
        let mut builder = native_tls::TlsConnector::builder();
        if no_cert_validation {
            builder.danger_accept_invalid_certs(true);
        }
        let connector = tokio_tls::TlsConnector::from(builder.build().unwrap());
        connector.connect(&domain2, tcp).map_err(Error::from)
    })
    .and_then(|tls| hyper::client::conn::handshake(tls).map_err(Error::from))
}

// convenience helper for non-Deserialize data...
fn required_string_member(value: &serde_json::Value, member: &str) -> Result<String, Error> {
    Ok(value
        .get(member)
        .ok_or_else(|| format_err!("missing '{}' in response", member))?
        .as_str()
        .ok_or_else(|| format_err!("invalid data type for '{}' in response", member))?
        .to_string())
}

struct Auth {
    ticket: String,
    token: String,
}

// Create a future which logs in on a proxmox backup server and yields an Auth struct.
fn login(
    domain: &str,
    port: u16,
    no_cert_validation: bool,
    urlbase: &str,
    user: String,
    pass: String,
) -> impl Future<Item = Auth, Error = Error> {
    let formdata = Body::from(
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("username", &{ user })
            .append_pair("password", &{ pass })
            .finish(),
    );

    let urlbase = urlbase.to_string();
    connect(domain, port, no_cert_validation)
        .and_then(move |(mut client, conn)| {
            let req = Request::builder()
                .method("POST")
                .uri(format!("{}/access/ticket", urlbase))
                .header("Content-type", "application/x-www-form-urlencoded")
                .body(formdata)?;
            Ok((client.send_request(req), conn))
        })
        .and_then(|(res, conn)| {
            let mut conn = Some(conn);
            res.map(|res| {
                res.into_body()
                    .concat2()
                    .map_err(Error::from)
                    .and_then(|data| {
                        let data: serde_json::Value = serde_json::from_slice(&data)?;
                        let data = data
                            .get("data")
                            .ok_or_else(|| format_err!("missing 'data' in response"))?;
                        let ticket = required_string_member(data, "ticket")?;
                        let token = required_string_member(data, "CSRFPreventionToken")?;

                        Ok(Auth { ticket, token })
                    })
            })
            .join(poll_fn(move || {
                try_ready!(conn.as_mut().unwrap().poll_without_shutdown());
                Ok(Async::Ready(conn.take().unwrap()))
            }))
            .map_err(Error::from)
        })
        .and_then(|(res, _conn)| res)
}

// Factored out protocol switching future: Takes a Response future and a connection and verifies
// its returned headers and protocol values. Yields a Response and the connection.
fn switch_protocols(
    res: hyper::client::conn::ResponseFuture,
    conn: HyperConnType,
) -> impl Future<Item = (Result<Response<Body>, Error>, HyperConnType), Error = Error> {
    let mut conn = Some(conn);
    res.map(|res| {
        if res.status() != StatusCode::SWITCHING_PROTOCOLS {
            bail!("unexpected status code - expected SwitchingProtocols");
        }
        let upgrade = match res.headers().get("Upgrade") {
            None => bail!("missing upgrade header in server response!"),
            Some(u) => u,
        };
        if upgrade != "proxmox-backup-protocol-1" {
            match upgrade.to_str() {
                Ok(s) => bail!("unexpected upgrade protocol type received: {}", s),
                _ => bail!("unexpected upgrade protocol type received"),
            }
        }
        Ok(res)
    })
    .map_err(Error::from)
    .join(poll_fn(move || {
        try_ready!(conn.as_mut().unwrap().poll_without_shutdown());
        Ok(Async::Ready(conn.take().unwrap()))
    }))
}

// Base for the two uploaders: DynamicIndexUploader and FixedIndexUploader:
struct UploaderBase<S: AsyncRead + AsyncWrite> {
    client: Option<PmxClient<S>>,
    wait_id: Option<StreamId>,
}

impl<S: AsyncRead + AsyncWrite> UploaderBase<S> {
    pub fn new(client: PmxClient<S>) -> Self {
        Self {
            client: Some(client),
            wait_id: None,
        }
    }

    pub fn create_backup(
        &mut self,
        index_type: IndexType,
        backup_type: &str,
        backup_id: &str,
        backup_timestamp: i64,
        filename: &str,
        chunk_size: usize,
        file_size: Option<u64>,
    ) -> Result<BackupStream, Error> {
        if self.wait_id.is_some() {
            bail!("create_backup cannot be called while awaiting a response");
        }

        let backup_stream = self.client.as_mut().unwrap().create_backup(
            index_type,
            backup_type,
            backup_id,
            backup_timestamp,
            filename,
            chunk_size,
            file_size,
            true,
        )?;
        self.wait_id = Some(backup_stream.into());
        Ok(backup_stream)
    }

    pub fn poll_ack(&mut self) -> Poll<(), Error> {
        if let Some(id) = self.wait_id {
            if self.client.as_mut().unwrap().wait_for_id(id)? {
                self.wait_id = None;
            } else {
                return Ok(Async::NotReady);
            }
        }
        return Ok(Async::Ready(()));
    }

    pub fn poll_send(&mut self) -> Poll<(), Error> {
        match self.client.as_mut().unwrap().poll_send()? {
            Some(false) => Ok(Async::NotReady),
            _ => Ok(Async::Ready(())),
        }
    }

    pub fn upload_chunk(
        &mut self,
        info: &ChunkEntry,
        chunk: &[u8],
    ) -> Result<Option<StreamId>, Error> {
        self.client.as_mut().unwrap().upload_chunk(info, chunk)
    }

    pub fn continue_upload_chunk(&mut self, chunk: &[u8]) -> Result<Option<StreamId>, Error> {
        let res = self.client.as_mut().unwrap().continue_upload_chunk(chunk)?;
        if let Some(id) = res {
            self.wait_id = Some(id);
        }
        Ok(res)
    }

    pub fn finish_backup(&mut self, stream: BackupStream) -> Result<(), Error> {
        let (ack, name, _done) = self.client.as_mut().unwrap().finish_backup(stream)?;
        println!("Server created file: {}", name);
        self.wait_id = Some(ack);
        Ok(())
    }

    pub fn take_client(&mut self) -> Option<PmxClient<S>> {
        self.client.take()
    }
}

// Future which creates a backup with a dynamic file:
struct DynamicIndexUploader<C: AsyncRead, S: AsyncRead + AsyncWrite> {
    base: UploaderBase<S>,
    chunks: ChunkStream<C>,
    current_chunk: Option<ChunkEntry>,
    backup_stream: Option<BackupStream>,
}

impl<C: AsyncRead, S: AsyncRead + AsyncWrite> DynamicIndexUploader<C, S> {
    pub fn new(
        client: PmxClient<S>,
        chunks: ChunkStream<C>,
        backup_type: &str,
        backup_id: &str,
        backup_timestamp: i64,
        filename: &str,
        chunk_size: usize,
    ) -> Result<Self, Error> {
        let mut base = UploaderBase::new(client);
        let stream = base.create_backup(
            IndexType::Dynamic,
            backup_type,
            backup_id,
            backup_timestamp,
            filename,
            chunk_size,
            None,
        )?;
        Ok(Self {
            base,
            chunks,
            current_chunk: None,
            backup_stream: Some(stream),
        })
    }

    fn get_chunk<'a>(chunks: &'a mut ChunkStream<C>) -> Poll<Option<&'a [u8]>, Error> {
        match chunks.get() {
            Ok(Some(None)) => Ok(Async::Ready(None)),
            Ok(Some(Some(chunk))) => Ok(Async::Ready(Some(chunk))),
            Ok(None) => return Ok(Async::NotReady),
            Err(e) => return Err(e),
        }
    }

    fn finished_chunk(&mut self) -> Result<(), Error> {
        self.base.client.as_mut().unwrap().dynamic_chunk(
            self.backup_stream.unwrap(),
            self.current_chunk.as_ref().unwrap(),
        )?;

        self.current_chunk = None;
        self.chunks.next();
        Ok(())
    }
}

impl<C: AsyncRead, S: AsyncRead + AsyncWrite> Future for DynamicIndexUploader<C, S> {
    type Item = PmxClient<S>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Error> {
        loop {
            // Process our upload queue if we have one:
            try_ready!(self.base.poll_send());

            // If we have a chunk in-flight, wait for acknowledgement:
            try_ready!(self.base.poll_ack());

            // Get our current chunk:
            let chunk = match try_ready!(Self::get_chunk(&mut self.chunks)) {
                Some(chunk) => chunk,
                None => match self.backup_stream.take() {
                    Some(stream) => {
                        self.base.finish_backup(stream)?;
                        continue;
                    }
                    None => return Ok(Async::Ready(self.base.take_client().unwrap())),
                },
            };

            // If the current chunk is in-flight just poll the upload:
            if self.current_chunk.is_some() {
                if self.base.continue_upload_chunk(chunk)?.is_some() {
                    self.finished_chunk()?;
                }
                continue;
            }

            let client = self.base.client.as_ref().unwrap();

            // We got a new chunk, see if we need to upload it:
            self.current_chunk = Some(ChunkEntry::from_data(chunk));
            let entry = self.current_chunk.as_ref().unwrap();
            if client.is_chunk_available(entry) {
                eprintln!("Already available: {}", entry.digest_to_hex());
                self.finished_chunk()?;
            } else {
                eprintln!("New chunk: {}, size {}", entry.digest_to_hex(), entry.len());
                match self.base.upload_chunk(entry, chunk)? {
                    Some(_id) => {
                        eprintln!("Finished right away!");
                        self.finished_chunk()?;
                    }
                    None => {
                        // Send-buffer filled up, start polling the upload process.
                        continue;
                    }
                }
            }
        }
    }
}

struct FixedIndexUploader<T: AsyncRead, S: AsyncRead + AsyncWrite> {
    base: UploaderBase<S>,
    input: T,
    backup_stream: Option<BackupStream>,
    current_chunk: Option<ChunkEntry>,
    chunk_size: usize,
    index: usize,
    buffer: Vec<u8>,
    eof: bool,
}

impl<T: AsyncRead, S: AsyncRead + AsyncWrite> FixedIndexUploader<T, S> {
    pub fn new(
        client: PmxClient<S>,
        input: T,
        backup_type: &str,
        backup_id: &str,
        backup_timestamp: i64,
        filename: &str,
        chunk_size: usize,
        file_size: u64,
    ) -> Result<Self, Error> {
        let mut base = UploaderBase::new(client);
        let stream = base.create_backup(
            IndexType::Fixed,
            backup_type,
            backup_id,
            backup_timestamp,
            filename,
            chunk_size,
            Some(file_size),
        )?;
        Ok(Self {
            base,
            input,
            backup_stream: Some(stream),
            current_chunk: None,
            chunk_size,
            index: 0,
            buffer: Vec::with_capacity(chunk_size),
            eof: false,
        })
    }

    fn fill_chunk(&mut self) -> Poll<bool, io::Error> {
        let mut pos = self.buffer.len();

        // we hit eof and we want the next chunk, return false:
        if self.eof && pos == 0 {
            return Ok(Async::Ready(false));
        }

        // we still have a full chunk right now:
        if pos == self.chunk_size {
            return Ok(Async::Ready(true));
        }

        // fill it up:
        unsafe {
            self.buffer.set_len(self.chunk_size);
        }
        let res = loop {
            match self.input.poll_read(&mut self.buffer[pos..]) {
                Err(e) => break Err(e),
                Ok(Async::NotReady) => break Ok(Async::NotReady),
                Ok(Async::Ready(got)) => {
                    if got == 0 {
                        self.eof = true;
                        break Ok(Async::Ready(true));
                    }
                    pos += got;
                    if pos == self.chunk_size {
                        break Ok(Async::Ready(true));
                    }
                    // read more...
                }
            }
        };
        unsafe {
            self.buffer.set_len(pos);
        }
        res
    }

    fn finished_chunk(&mut self) -> Result<(), Error> {
        self.base.client.as_mut().unwrap().fixed_data(
            self.backup_stream.unwrap(),
            self.index,
            self.current_chunk.as_ref().unwrap(),
        )?;
        self.index += 1;
        self.current_chunk = None;
        unsafe {
            // This is how we tell fill_chunk() that it needs to read new data
            self.buffer.set_len(0);
        }
        Ok(())
    }
}

impl<T: AsyncRead, S: AsyncRead + AsyncWrite> Future for FixedIndexUploader<T, S> {
    type Item = PmxClient<S>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Error> {
        loop {
            // Process our upload queue if we have one:
            try_ready!(self.base.poll_send());

            // If we have a chunk in-flight, wait for acknowledgement:
            try_ready!(self.base.poll_ack());

            // Get our current chunk:
            if !try_ready!(self.fill_chunk()) {
                match self.backup_stream.take() {
                    Some(stream) => {
                        self.base.finish_backup(stream)?;
                        continue;
                    }
                    None => {
                        return Ok(Async::Ready(self.base.take_client().unwrap()));
                    }
                }
            };

            let chunk = &self.buffer[..];

            // If the current chunk is in-flight just poll the upload:
            if self.current_chunk.is_some() {
                if self.base.continue_upload_chunk(chunk)?.is_some() {
                    self.finished_chunk()?;
                }
                continue;
            }

            let client = self.base.client.as_ref().unwrap();

            // We got a new chunk, see if we need to upload it:
            self.current_chunk = Some(ChunkEntry::from_data(chunk));
            let entry = self.current_chunk.as_ref().unwrap();
            if client.is_chunk_available(entry) {
                eprintln!("Already available: {}", entry.digest_to_hex());
                self.finished_chunk()?;
            } else {
                eprintln!("New chunk: {}, size {}", entry.digest_to_hex(), entry.len());
                match self.base.upload_chunk(entry, chunk)? {
                    Some(_id) => {
                        eprintln!("Finished right away!");
                        self.finished_chunk()?;
                    }
                    None => {
                        // Send-buffer filled up, start polling the upload process.
                        continue;
                    }
                }
            }
        }
    }
}

// Helper-Future for waiting for a polling method on proxmox_protocol::Client to complete:
struct ClientWaitFuture<S: AsyncRead + AsyncWrite>(
    Option<PmxClient<S>>,
    fn(&mut PmxClient<S>) -> Result<bool, Error>,
);

impl<S: AsyncRead + AsyncWrite> Future for ClientWaitFuture<S> {
    type Item = PmxClient<S>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if (self.1)(self.0.as_mut().unwrap())? {
            Ok(Async::Ready(self.0.take().unwrap()))
        } else {
            Ok(Async::NotReady)
        }
    }
}

// Trait to provide Futures for some proxmox_protocol::Client methods:
trait ClientOps<S: AsyncRead + AsyncWrite> {
    fn poll_handshake(self) -> ClientWaitFuture<S>;
    fn poll_hashes(self, file: &str) -> Result<ClientWaitFuture<S>, Error>;
}

impl<S: AsyncRead + AsyncWrite> ClientOps<S> for PmxClient<S> {
    fn poll_handshake(self) -> ClientWaitFuture<S> {
        ClientWaitFuture(Some(self), PmxClient::<S>::wait_for_handshake)
    }

    fn poll_hashes(mut self, name: &str) -> Result<ClientWaitFuture<S>, Error> {
        self.query_hashes(name)?;
        Ok(ClientWaitFuture(Some(self), PmxClient::<S>::wait_for_hashes))
    }
}

// CLI helper.
fn require_arg(args: &mut dyn Iterator<Item = String>, name: &str) -> String {
    match args.next() {
        Some(arg) => arg,
        None => {
            eprintln!("missing required argument: {}", name);
            exit(1);
        }
    }
}

fn main() {
    // Usage:
    //   ./proxmox-protocol-testclient <type> <id> <filename> [<optional old-file>]
    //
    // This will query the remote server for a list of chunks in <old-file> if the argument was
    // provided, otherwise assumes all chunks are new.

    let mut args = std::env::args().skip(1);
    let mut repo = require_arg(&mut args, "repository");
    let use_fixed_chunks = if repo == "--fixed" {
        repo = require_arg(&mut args, "repository");
        true
    } else {
        false
    };
    let backup_type = require_arg(&mut args, "backup-type");
    let backup_id = require_arg(&mut args, "backup-id");
    let filename = require_arg(&mut args, "backup-file-name");
    // optional previous backup:
    let previous = args.next().map(|s| s.to_string());

    let repo: BackupRepository = match repo.parse() {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("error parsing repository: {}", e);
            exit(1);
        }
    };

    let backup_time = Utc::now().timestamp();
    // Or fake the time to verify we cannot create an already existing backup:
    //let backup_time = Utc::today().and_hms(3, 25, 55);

    println!(
        "Uploading file `{}`, type {}, id: {}",
        filename, backup_type, backup_id
    );

    let no_cert_validation = true; // FIXME
    let domain = repo.host().to_owned();
    let port = 8007;
    let address = format!("{}:{}", domain, port);
    let urlbase = format!("https://{}/api2/json", address);

    let user = repo.user().to_string();
    let pass = match proxmox_backup::tools::tty::read_password("Password: ")
        .and_then(|x| String::from_utf8(x).map_err(Error::from))
    {
        Ok(pass) => pass,
        Err(e) => {
            eprintln!("error getting password: {}", e);
            exit(1);
        }
    };
    let store = repo.store().to_owned();

    let stream = File::open(filename.clone())
        .map_err(Error::from)
        .join(login(
            &domain,
            port,
            no_cert_validation,
            &urlbase,
            user,
            pass,
        ))
        .and_then(move |(file, auth)| {
            ok((file, auth)).join(connect(&domain, port, no_cert_validation))
        })
        .and_then(move |((file, auth), (mut client, conn))| {
            let req = Request::builder()
                .method("GET")
                .uri(format!("{}/admin/datastore/{}/test-upload", urlbase, store))
                .header("Cookie", format!("PBSAuthCookie={}", auth.ticket))
                .header("CSRFPreventionToken", auth.token)
                .header("Connection", "Upgrade")
                .header("Upgrade", "proxmox-backup-protocol-1")
                .body(Body::empty())?;
            Ok((file, client.send_request(req), conn))
        })
        .and_then(|(file, res, conn)| ok(file).join(switch_protocols(res, conn)))
        .and_then(|(file, (_, conn))| {
            let client = PmxClient::new(conn.into_parts().io);
            file.metadata()
                .map_err(Error::from)
                .join(client.poll_handshake())
        })
        .and_then(move |((file, meta), client)| {
            eprintln!("Server said hello");
            // 2 possible futures of distinct types need an explicit cast to Box<dyn Future>...
            let fut: Box<dyn Future<Item = _, Error = _> + Send> =
                if let Some(previous) = previous {
                    let query = client.poll_hashes(&previous)?;
                    Box::new(ok((file, meta)).join(query))
                } else {
                    Box::new(ok(((file, meta), client)))
                };
            Ok(fut)
        })
        .flatten()
        .and_then(move |((file, meta), client)| {
            eprintln!("starting uploader...");
            let uploader: Box<dyn Future<Item = _, Error = _> + Send> = if use_fixed_chunks {
                Box::new(FixedIndexUploader::new(
                    client,
                    file,
                    &backup_type,
                    &backup_id,
                    backup_time,
                    &filename,
                    4 * 1024 * 1024,
                    meta.len(),
                )?)
            } else {
                let chunker = ChunkStream::new(file);
                Box::new(DynamicIndexUploader::new(
                    client,
                    chunker,
                    &backup_type,
                    &backup_id,
                    backup_time,
                    &filename,
                    4 * 1024 * 1024,
                )?)
            };
            Ok(uploader)
        })
        .flatten();

    let stream = stream
        .and_then(move |_client| {
            println!("Done");
            Ok(())
        })
        .map_err(|e| eprintln!("error: {}", e));
    hyper::rt::run(stream);
}
