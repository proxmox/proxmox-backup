use std::collections::HashSet;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use failure::*;
use futures::*;
use futures::stream::Stream;
use http::Uri;
use http::header::HeaderValue;
use http::{Request, Response};
use hyper::Body;
use hyper::client::Client;
use openssl::ssl::{SslConnector, SslMethod};
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, oneshot};
use url::percent_encoding::{percent_encode,  DEFAULT_ENCODE_SET};
use xdg::BaseDirectories;

use proxmox::tools::{
    digest_to_hex,
    fs::{file_get_json, file_set_contents},
};

use super::merge_known_chunks::{MergedChunkInfo, MergeKnownChunks};
use super::pipe_to_stream::PipeToSendStream;
use crate::backup::*;
use crate::tools::futures::{cancellable, Canceller};
use crate::tools::{self, BroadcastFuture, tty};

#[derive(Clone)]
pub struct AuthInfo {
    username: String,
    ticket: String,
    token: String,
}

/// HTTP(S) API client
pub struct HttpClient {
    client: Client<hyper_openssl::HttpsConnector<hyper::client::HttpConnector>>,
    server: String,
    auth: BroadcastFuture<AuthInfo>,
}

/// Delete stored ticket data (logout)
pub fn delete_ticket_info(server: &str, username: &str) -> Result<(), Error> {

    let base = BaseDirectories::with_prefix("proxmox-backup")?;

    // usually /run/user/<uid>/...
    let path = base.place_runtime_file("tickets")?;

    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);

    let mut data = file_get_json(&path, Some(json!({})))?;

    if let Some(map) = data[server].as_object_mut() {
        map.remove(username);
    }

    file_set_contents(path, data.to_string().as_bytes(), Some(mode))?;

    Ok(())
}

fn store_ticket_info(server: &str, username: &str, ticket: &str, token: &str) -> Result<(), Error> {

    let base = BaseDirectories::with_prefix("proxmox-backup")?;

    // usually /run/user/<uid>/...
    let path = base.place_runtime_file("tickets")?;

    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);

    let mut data = file_get_json(&path, Some(json!({})))?;

    let now = Utc::now().timestamp();

    data[server][username] = json!({ "timestamp": now, "ticket": ticket, "token": token});

    let mut new_data = json!({});

    let ticket_lifetime = tools::ticket::TICKET_LIFETIME - 60;

    let empty = serde_json::map::Map::new();
    for (server, info) in data.as_object().unwrap_or(&empty) {
        for (_user, uinfo) in info.as_object().unwrap_or(&empty) {
            if let Some(timestamp) = uinfo["timestamp"].as_i64() {
                let age = now - timestamp;
                if age < ticket_lifetime {
                    new_data[server][username] = uinfo.clone();
                }
            }
        }
    }

    file_set_contents(path, new_data.to_string().as_bytes(), Some(mode))?;

    Ok(())
}

fn load_ticket_info(server: &str, username: &str) -> Option<(String, String)> {
    let base = BaseDirectories::with_prefix("proxmox-backup").ok()?;

    // usually /run/user/<uid>/...
    let path = base.place_runtime_file("tickets").ok()?;
    let data = file_get_json(&path, None).ok()?;
    let now = Utc::now().timestamp();
    let ticket_lifetime = tools::ticket::TICKET_LIFETIME - 60;
    let uinfo = data[server][username].as_object()?;
    let timestamp = uinfo["timestamp"].as_i64()?;
    let age = now - timestamp;

    if age < ticket_lifetime {
        let ticket = uinfo["ticket"].as_str()?;
        let token = uinfo["token"].as_str()?;
        Some((ticket.to_owned(), token.to_owned()))
    } else {
        None
    }
}

impl HttpClient {

    pub fn new(server: &str, username: &str) -> Result<Self, Error> {
        let client = Self::build_client();

        let password = if let Some((ticket, _token)) = load_ticket_info(server, username) {
            ticket
        } else {
            Self::get_password(&username)?
        };

        let login = Self::credentials(client.clone(), server.to_owned(), username.to_owned(), password);

        Ok(Self {
            client,
            server: String::from(server),
            auth: BroadcastFuture::new(login),
        })
    }

    /// Login future
    ///
    /// Login is done on demand, so this is onyl required if you need
    /// access to authentication data in 'AuthInfo'.
    pub fn login(&self) -> impl Future<Output = Result<AuthInfo, Error>> {
        self.auth.listen()
    }

    fn get_password(_username: &str) -> Result<String, Error> {
        use std::env::VarError::*;
        match std::env::var("PBS_PASSWORD") {
            Ok(p) => return Ok(p),
            Err(NotUnicode(_)) => bail!("PBS_PASSWORD contains bad characters"),
            Err(NotPresent) => {
                // Try another method
            }
        }

        // If we're on a TTY, query the user for a password
        if tty::stdin_isatty() {
            return Ok(String::from_utf8(tty::read_password("Password: ")?)?);
        }

        bail!("no password input mechanism available");
    }

    fn build_client() -> Client<hyper_openssl::HttpsConnector<hyper::client::HttpConnector>> {

        let mut ssl_connector_builder = SslConnector::builder(SslMethod::tls()).unwrap();

        ssl_connector_builder.set_verify(openssl::ssl::SslVerifyMode::NONE); // fixme!

        let mut httpc = hyper::client::HttpConnector::new(1);
        httpc.set_nodelay(true); // important for h2 download performance!
        httpc.set_recv_buffer_size(Some(1024*1024)); //important for h2 download performance!
        httpc.enforce_http(false); // we want https...

        let https = hyper_openssl::HttpsConnector::with_connector(httpc,  ssl_connector_builder).unwrap();

        Client::builder()
        //.http2_initial_stream_window_size( (1 << 31) - 2)
        //.http2_initial_connection_window_size( (1 << 31) - 2)
            .build::<_, Body>(https)
    }

    pub fn request(&self, mut req: Request<Body>) -> impl Future<Output = Result<Value, Error>> {

        let login = self.auth.listen();

        let client = self.client.clone();

        login.and_then(move |auth| {

            let enc_ticket = format!("PBSAuthCookie={}", percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET));
            req.headers_mut().insert("Cookie", HeaderValue::from_str(&enc_ticket).unwrap());
            req.headers_mut().insert("CSRFPreventionToken", HeaderValue::from_str(&auth.token).unwrap());

            let request = Self::api_request(client, req);

            request
        })
    }

    pub fn get(
        &self,
        path: &str,
        data: Option<Value>,
    ) -> impl Future<Output = Result<Value, Error>> {
        let req = Self::request_builder(&self.server, "GET", path, data).unwrap();
        self.request(req)
    }

    pub fn delete(
        &mut self,
        path: &str,
        data: Option<Value>,
    ) -> impl Future<Output = Result<Value, Error>> {
        let req = Self::request_builder(&self.server, "DELETE", path, data).unwrap();
        self.request(req)
    }

    pub fn post(
        &mut self,
        path: &str,
        data: Option<Value>,
    ) -> impl Future<Output = Result<Value, Error>> {
        let req = Self::request_builder(&self.server, "POST", path, data).unwrap();
        self.request(req)
    }

    pub fn download<W: Write + Send + 'static>(
        &mut self,
        path: &str,
        output: W,
    ) ->  impl Future<Output = Result<W, Error>> {
        let mut req = Self::request_builder(&self.server, "GET", path, None).unwrap();

        let login = self.auth.listen();

        let client = self.client.clone();

        login.and_then(move |auth| {

            let enc_ticket = format!("PBSAuthCookie={}", percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET));
            req.headers_mut().insert("Cookie", HeaderValue::from_str(&enc_ticket).unwrap());

            client.request(req)
                .map_err(Error::from)
                .and_then(|resp| {
                    let status = resp.status();
                    if !status.is_success() {
                        future::Either::Left(
                            HttpClient::api_response(resp)
                                .map(|_| Err(format_err!("unknown error")))
                        )
                    } else {
                        future::Either::Right(
                            resp.into_body()
                                .map_err(Error::from)
                                .try_fold(output, move |mut acc, chunk| async move {
                                    acc.write_all(&chunk)?;
                                    Ok::<_, Error>(acc)
                                })
                        )
                    }
                })
        })
    }

    pub fn upload(
        &mut self,
        content_type: &str,
        body: Body,
        path: &str,
        data: Option<Value>,
    ) -> impl Future<Output = Result<Value, Error>> {

        let path = path.trim_matches('/');
        let mut url = format!("https://{}:8007/{}", &self.server, path);

        if let Some(data) = data {
            let query = tools::json_object_to_query(data).unwrap();
            url.push('?');
            url.push_str(&query);
        }

        let url: Uri = url.parse().unwrap();

        let req = Request::builder()
            .method("POST")
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header("Content-Type", content_type)
            .body(body).unwrap();

        self.request(req)
    }

    pub fn start_backup(
        &self,
        datastore: &str,
        backup_type: &str,
        backup_id: &str,
        backup_time: DateTime<Utc>,
        debug: bool,
    ) -> impl Future<Output = Result<Arc<BackupClient>, Error>> {

        let param = json!({
            "backup-type": backup_type,
            "backup-id": backup_id,
            "backup-time": backup_time.timestamp(),
            "store": datastore,
            "debug": debug
        });

        let req = Self::request_builder(&self.server, "GET", "/api2/json/backup", Some(param)).unwrap();

        self.start_h2_connection(req, String::from(PROXMOX_BACKUP_PROTOCOL_ID_V1!()))
            .map_ok(|(h2, canceller)| BackupClient::new(h2, canceller))
    }

    pub fn start_backup_reader(
        &self,
        datastore: &str,
        backup_type: &str,
        backup_id: &str,
        backup_time: DateTime<Utc>,
        debug: bool,
    ) -> impl Future<Output = Result<Arc<BackupReader>, Error>> {

        let param = json!({
            "backup-type": backup_type,
            "backup-id": backup_id,
            "backup-time": backup_time.timestamp(),
            "store": datastore,
            "debug": debug,
        });
        let req = Self::request_builder(&self.server, "GET", "/api2/json/reader", Some(param)).unwrap();

        self.start_h2_connection(req, String::from(PROXMOX_BACKUP_READER_PROTOCOL_ID_V1!()))
            .map_ok(|(h2, canceller)| BackupReader::new(h2, canceller))
    }

    pub fn start_h2_connection(
        &self,
        mut req: Request<Body>,
        protocol_name: String,
    ) -> impl Future<Output = Result<(H2Client, Canceller), Error>> {

        let login = self.auth.listen();
        let client = self.client.clone();

        login.and_then(move |auth| {

            let enc_ticket = format!("PBSAuthCookie={}", percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET));
            req.headers_mut().insert("Cookie", HeaderValue::from_str(&enc_ticket).unwrap());
            req.headers_mut().insert("UPGRADE", HeaderValue::from_str(&protocol_name).unwrap());

            client.request(req)
                .map_err(Error::from)
                .and_then(|resp| {

                    let status = resp.status();
                    if status != http::StatusCode::SWITCHING_PROTOCOLS {
                        future::Either::Left(
                            Self::api_response(resp)
                                .map(|_| Err(format_err!("unknown error")))
                        )
                    } else {
                        future::Either::Right(
                            resp
                                .into_body()
                                .on_upgrade()
                                .map_err(Error::from)
                        )
                    }
                })
                .and_then(|upgraded| {
                   let max_window_size = (1 << 31) - 2;

                    h2::client::Builder::new()
                        .initial_connection_window_size(max_window_size)
                        .initial_window_size(max_window_size)
                        .max_frame_size(4*1024*1024)
                        .handshake(upgraded)
                        .map_err(Error::from)
                })
                .and_then(|(h2, connection)| async move {
                    let connection = connection
                        .map_err(|_| panic!("HTTP/2.0 connection failed"));

                    let (connection, canceller) = cancellable(connection)?;
                    // A cancellable future returns an Option which is None when cancelled and
                    // Some when it finished instead, since we don't care about the return type we
                    // need to map it away:
                    let connection = connection.map(|_| ());

                    // Spawn a new task to drive the connection state
                    hyper::rt::spawn(connection);

                    // Wait until the `SendRequest` handle has available capacity.
                    let c = h2.ready().await?;
                    Ok((H2Client::new(c), canceller))
                }.boxed())
        })
    }

    fn credentials(
        client: Client<hyper_openssl::HttpsConnector<hyper::client::HttpConnector>>,
        server: String,
        username: String,
        password: String,
    ) -> Box<dyn Future<Output = Result<AuthInfo, Error>> + Send> {
        Box::new(async move {
            let data = json!({ "username": username, "password": password });
            let req = Self::request_builder(&server, "POST", "/api2/json/access/ticket", Some(data)).unwrap();
            let cred = Self::api_request(client, req).await?;
            let auth = AuthInfo {
                username: cred["data"]["username"].as_str().unwrap().to_owned(),
                ticket: cred["data"]["ticket"].as_str().unwrap().to_owned(),
                token: cred["data"]["CSRFPreventionToken"].as_str().unwrap().to_owned(),
            };

            let _ = store_ticket_info(&server, &auth.username, &auth.ticket, &auth.token);

            Ok(auth)
        })
    }

    async fn api_response(response: Response<Body>) -> Result<Value, Error> {
        let status = response.status();
        let data = response
            .into_body()
            .try_concat()
            .await?;

        let text = String::from_utf8(data.to_vec()).unwrap();
        if status.is_success() {
            if text.len() > 0 {
                let value: Value = serde_json::from_str(&text)?;
                Ok(value)
            } else {
                Ok(Value::Null)
            }
        } else {
            bail!("HTTP Error {}: {}", status, text);
        }
    }

    fn api_request(
        client: Client<hyper_openssl::HttpsConnector<hyper::client::HttpConnector>>,
        req: Request<Body>
    ) -> impl Future<Output = Result<Value, Error>> {

        client.request(req)
            .map_err(Error::from)
            .and_then(Self::api_response)
    }

    pub fn request_builder(server: &str, method: &str, path: &str, data: Option<Value>) -> Result<Request<Body>, Error> {
        let path = path.trim_matches('/');
        let url: Uri = format!("https://{}:8007/{}", server, path).parse()?;

        if let Some(data) = data {
            if method == "POST" {
                let request = Request::builder()
                    .method(method)
                    .uri(url)
                    .header("User-Agent", "proxmox-backup-client/1.0")
                    .header(hyper::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(data.to_string()))?;
                return Ok(request);
            } else {
                let query = tools::json_object_to_query(data)?;
                let url: Uri = format!("https://{}:8007/{}?{}", server, path, query).parse()?;
                let request = Request::builder()
                    .method(method)
                    .uri(url)
                    .header("User-Agent", "proxmox-backup-client/1.0")
                    .header(hyper::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::empty())?;
                return Ok(request);
            }
        }

        let request = Request::builder()
            .method(method)
            .uri(url)
            .header("User-Agent", "proxmox-backup-client/1.0")
            .header(hyper::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::empty())?;

        Ok(request)
    }
}


pub struct BackupReader {
    h2: H2Client,
    canceller: Canceller,
}

impl Drop for BackupReader {

    fn drop(&mut self) {
        self.canceller.cancel();
    }
}

impl BackupReader {

    pub fn new(h2: H2Client, canceller: Canceller) -> Arc<Self> {
        Arc::new(Self { h2, canceller: canceller })
    }

    pub fn get(
        &self,
        path: &str,
        param: Option<Value>,
    ) -> impl Future<Output = Result<Value, Error>> {
        self.h2.get(path, param)
    }

    pub fn put(
        &self,
        path: &str,
        param: Option<Value>,
    ) -> impl Future<Output = Result<Value, Error>> {
        self.h2.put(path, param)
    }

    pub fn post(
        &self,
        path: &str,
        param: Option<Value>,
    ) -> impl Future<Output = Result<Value, Error>> {
        self.h2.post(path, param)
    }

    pub fn download<W: Write + Send + 'static>(
        &self,
        file_name: &str,
        output: W,
    ) -> impl Future<Output = Result<W, Error>> {
        let path = "download";
        let param = json!({ "file-name": file_name });
        self.h2.download(path, Some(param), output)
    }

    pub fn speedtest<W: Write + Send + 'static>(
        &self,
        output: W,
    ) -> impl Future<Output = Result<W, Error>> {
        self.h2.download("speedtest", None, output)
    }

    pub fn download_chunk<W: Write + Send + 'static>(
        &self,
        digest: &[u8; 32],
        output: W,
    ) -> impl Future<Output = Result<W, Error>> {
        let path = "chunk";
        let param = json!({ "digest": digest_to_hex(digest) });
        self.h2.download(path, Some(param), output)
    }

    pub fn force_close(self) {
        self.canceller.cancel();
    }
}

pub struct BackupClient {
    h2: H2Client,
    canceller: Canceller,
}

impl Drop for BackupClient {

    fn drop(&mut self) {
        self.canceller.cancel();
    }
}

pub struct BackupStats {
    pub size: u64,
    pub csum: [u8; 32],
}

impl BackupClient {
    pub fn new(h2: H2Client, canceller: Canceller) -> Arc<Self> {
        Arc::new(Self { h2, canceller })
    }

    pub fn get(
        &self,
        path: &str,
        param: Option<Value>,
    ) -> impl Future<Output = Result<Value, Error>> {
        self.h2.get(path, param)
    }

    pub fn put(
        &self,
        path: &str,
        param: Option<Value>,
    ) -> impl Future<Output = Result<Value, Error>> {
        self.h2.put(path, param)
    }

    pub fn post(
        &self,
        path: &str,
        param: Option<Value>,
    ) -> impl Future<Output = Result<Value, Error>> {
        self.h2.post(path, param)
    }

    pub fn finish(self: Arc<Self>) -> impl Future<Output = Result<(), Error>> {
        self.h2.clone()
            .post("finish", None)
            .map_ok(move |_| {
                self.canceller.cancel();
            })
    }

    pub fn force_close(self) {
        self.canceller.cancel();
    }

    pub fn upload_blob<R: std::io::Read>(
        &self,
        mut reader: R,
        file_name: &str,
     ) -> impl Future<Output = Result<BackupStats, Error>> {

        let h2 = self.h2.clone();
        let file_name = file_name.to_owned();

        async move {
            let mut raw_data = Vec::new();
            // fixme: avoid loading into memory
            reader.read_to_end(&mut raw_data)?;

            let csum = openssl::sha::sha256(&raw_data);
            let param = json!({"encoded-size": raw_data.len(), "file-name": file_name });
            let size = raw_data.len() as u64; // fixme: should be decoded size instead??
            let _value = h2.upload("blob", Some(param), raw_data).await?;
            Ok(BackupStats { size, csum })
        }
    }

    pub fn upload_blob_from_data(
        &self,
        data: Vec<u8>,
        file_name: &str,
        crypt_config: Option<Arc<CryptConfig>>,
        compress: bool,
        sign_only: bool,
     ) -> impl Future<Output = Result<BackupStats, Error>> {

        let h2 = self.h2.clone();
        let file_name = file_name.to_owned();
        let size = data.len() as u64;

        async move {
            let blob = if let Some(crypt_config) = crypt_config {
                if sign_only {
                    DataBlob::create_signed(&data, crypt_config, compress)?
                } else {
                    DataBlob::encode(&data, Some(crypt_config.clone()), compress)?
                }
            } else {
                DataBlob::encode(&data, None, compress)?
            };

            let raw_data = blob.into_inner();

            let csum = openssl::sha::sha256(&raw_data);
            let param = json!({"encoded-size": raw_data.len(), "file-name": file_name });
            let _value = h2.upload("blob", Some(param), raw_data).await?;
            Ok(BackupStats { size, csum })
        }
    }

    pub fn upload_blob_from_file<P: AsRef<std::path::Path>>(
        &self,
        src_path: P,
        file_name: &str,
        crypt_config: Option<Arc<CryptConfig>>,
        compress: bool,
     ) -> impl Future<Output = Result<BackupStats, Error>> {

        let h2 = self.h2.clone();
        let file_name = file_name.to_owned();
        let src_path = src_path.as_ref().to_owned();

        async move {
            let mut file = tokio::fs::File::open(src_path.clone())
                .await
                .map_err(move |err| format_err!("unable to open file {:?} - {}", src_path, err))?;

            let mut contents = Vec::new();
            file.read_to_end(&mut contents).await.map_err(Error::from)?;

            let size: u64 = contents.len() as u64;
            let blob = DataBlob::encode(&contents, crypt_config, compress)?;
            let raw_data = blob.into_inner();
            let csum = openssl::sha::sha256(&raw_data);
            let param = json!({
                "encoded-size": raw_data.len(),
                "file-name": file_name,
            });
            h2.upload("blob", Some(param), raw_data).await?;
            Ok(BackupStats { size, csum })
        }
    }

    pub fn upload_stream(
        &self,
        archive_name: &str,
        stream: impl Stream<Item = Result<bytes::BytesMut, Error>>,
        prefix: &str,
        fixed_size: Option<u64>,
        crypt_config: Option<Arc<CryptConfig>>,
    ) -> impl Future<Output = Result<BackupStats, Error>> {
        let known_chunks = Arc::new(Mutex::new(HashSet::new()));

        let mut param = json!({ "archive-name": archive_name });
        if let Some(size) = fixed_size {
            param["size"] = size.into();
        }

        let index_path = format!("{}_index", prefix);
        let close_path = format!("{}_close", prefix);

        let prefix = prefix.to_owned();

        let h2 = self.h2.clone();

        let download_future =
            Self::download_chunk_list(h2.clone(), &index_path, archive_name, known_chunks.clone());

        async move {
            download_future.await?;

            let wid = h2.post(&index_path, Some(param)).await?.as_u64().unwrap();

            let (chunk_count, size, _speed, csum) = Self::upload_chunk_info_stream(
                h2.clone(),
                wid,
                stream,
                &prefix,
                known_chunks.clone(),
                crypt_config,
            )
            .await?;

            let param = json!({
                "wid": wid ,
                "chunk-count": chunk_count,
                "size": size,
            });
            let _value = h2.post(&close_path, Some(param)).await?;
            Ok(BackupStats {
                size: size as u64,
                csum,
            })
        }
    }

    fn response_queue() -> (
        mpsc::Sender<h2::client::ResponseFuture>,
        oneshot::Receiver<Result<(), Error>>
    ) {
        let (verify_queue_tx, verify_queue_rx) = mpsc::channel(100);
        let (verify_result_tx, verify_result_rx) = oneshot::channel();

        hyper::rt::spawn(
            verify_queue_rx
                .map(Ok::<_, Error>)
                .try_for_each(|response: h2::client::ResponseFuture| {
                    response
                        .map_err(Error::from)
                        .and_then(H2Client::h2api_response)
                        .map_ok(|result| println!("RESPONSE: {:?}", result))
                        .map_err(|err| format_err!("pipelined request failed: {}", err))
                })
                .map(|result| {
                      let _ignore_closed_channel = verify_result_tx.send(result);
                })
        );

        (verify_queue_tx, verify_result_rx)
    }

    fn append_chunk_queue(h2: H2Client, wid: u64, path: String) -> (
        mpsc::Sender<(MergedChunkInfo, Option<h2::client::ResponseFuture>)>,
        oneshot::Receiver<Result<(), Error>>
    ) {
        let (verify_queue_tx, verify_queue_rx) = mpsc::channel(64);
        let (verify_result_tx, verify_result_rx) = oneshot::channel();

        let h2_2 = h2.clone();

        hyper::rt::spawn(
            verify_queue_rx
                .map(Ok::<_, Error>)
                .and_then(move |(merged_chunk_info, response): (MergedChunkInfo, Option<h2::client::ResponseFuture>)| {
                    match (response, merged_chunk_info) {
                        (Some(response), MergedChunkInfo::Known(list)) => {
                            future::Either::Left(
                                response
                                    .map_err(Error::from)
                                    .and_then(H2Client::h2api_response)
                                    .and_then(move |_result| {
                                        future::ok(MergedChunkInfo::Known(list))
                                    })
                            )
                        }
                        (None, MergedChunkInfo::Known(list)) => {
                            future::Either::Right(future::ok(MergedChunkInfo::Known(list)))
                        }
                        _ => unreachable!(),
                    }
                })
                .merge_known_chunks()
                .and_then(move |merged_chunk_info| {
                    match merged_chunk_info {
                        MergedChunkInfo::Known(chunk_list) => {
                            let mut digest_list = vec![];
                            let mut offset_list = vec![];
                            for (offset, digest) in chunk_list {
                                //println!("append chunk {} (offset {})", proxmox::tools::digest_to_hex(&digest), offset);
                                digest_list.push(digest_to_hex(&digest));
                                offset_list.push(offset);
                            }
                            println!("append chunks list len ({})", digest_list.len());
                            let param = json!({ "wid": wid, "digest-list": digest_list, "offset-list": offset_list });
                            let mut request = H2Client::request_builder("localhost", "PUT", &path, None).unwrap();
                            request.headers_mut().insert(hyper::header::CONTENT_TYPE,  HeaderValue::from_static("application/json"));
                            let param_data = bytes::Bytes::from(param.to_string().as_bytes());
                            let upload_data = Some(param_data);
                            h2_2.send_request(request, upload_data)
                                .and_then(move |response| {
                                    response
                                        .map_err(Error::from)
                                        .and_then(H2Client::h2api_response)
                                        .map_ok(|_| ())
                                })
                                .map_err(|err| format_err!("pipelined request failed: {}", err))
                        }
                        _ => unreachable!(),
                    }
                })
                .try_for_each(|_| future::ok(()))
                .map(|result| {
                      let _ignore_closed_channel = verify_result_tx.send(result);
                })
        );

        (verify_queue_tx, verify_result_rx)
    }

    fn download_chunk_list(
        h2: H2Client,
        path: &str,
        archive_name: &str,
        known_chunks: Arc<Mutex<HashSet<[u8;32]>>>,
    ) -> impl Future<Output = Result<(), Error>> {

        let param = json!({ "archive-name": archive_name });
        let request = H2Client::request_builder("localhost", "GET", path, Some(param)).unwrap();

        h2.send_request(request, None)
            .and_then(move |response| {
                response
                    .map_err(Error::from)
                    .and_then(move |resp| {
                        let status = resp.status();

                        if !status.is_success() {
                            future::Either::Left(
                                H2Client::h2api_response(resp)
                                    .map(|_| Err(format_err!("unknown error")))
                            )
                        } else {
                            future::Either::Right(future::ok(resp.into_body()))
                        }
                    })
                    .and_then(move |mut body| {

                        let mut release_capacity = body.release_capacity().clone();

                        DigestListDecoder::new(body.map_err(Error::from))
                            .try_for_each(move |chunk| {
                                let _ = release_capacity.release_capacity(chunk.len());
                                println!("GOT DOWNLOAD {}", digest_to_hex(&chunk));
                                known_chunks.lock().unwrap().insert(chunk);
                                futures::future::ok(())
                            })
                       })
            })
    }

    fn upload_chunk_info_stream(
        h2: H2Client,
        wid: u64,
        stream: impl Stream<Item = Result<bytes::BytesMut, Error>>,
        prefix: &str,
        known_chunks: Arc<Mutex<HashSet<[u8;32]>>>,
        crypt_config: Option<Arc<CryptConfig>>,
    ) -> impl Future<Output = Result<(usize, usize, usize, [u8; 32]), Error>> {

        let repeat = Arc::new(AtomicUsize::new(0));
        let repeat2 = repeat.clone();

        let stream_len = Arc::new(AtomicUsize::new(0));
        let stream_len2 = stream_len.clone();

        let append_chunk_path = format!("{}_index", prefix);
        let upload_chunk_path = format!("{}_chunk", prefix);

        let (upload_queue, upload_result) =
            Self::append_chunk_queue(h2.clone(), wid, append_chunk_path.to_owned());

        let start_time = std::time::Instant::now();

        let index_csum = Arc::new(Mutex::new(Some(openssl::sha::Sha256::new())));
        let index_csum_2 = index_csum.clone();

        stream
            .and_then(move |data| {

                let chunk_len = data.len();

                repeat.fetch_add(1, Ordering::SeqCst);
                let offset = stream_len.fetch_add(chunk_len, Ordering::SeqCst) as u64;

                let mut chunk_builder = DataChunkBuilder::new(data.as_ref())
                    .compress(true);

                if let Some(ref crypt_config) = crypt_config {
                    chunk_builder = chunk_builder.crypt_config(crypt_config);
                }

                let mut known_chunks = known_chunks.lock().unwrap();
                let digest = chunk_builder.digest();

                let mut guard = index_csum.lock().unwrap();
                let csum = guard.as_mut().unwrap();
                csum.update(&offset.to_le_bytes());
                csum.update(digest);

                let chunk_is_known = known_chunks.contains(digest);
                if chunk_is_known {
                    future::ok(MergedChunkInfo::Known(vec![(offset, *digest)]))
                } else {
                    known_chunks.insert(*digest);
                    future::ready(chunk_builder
                        .build()
                        .map(move |chunk| MergedChunkInfo::New(ChunkInfo {
                            chunk,
                            chunk_len: chunk_len as u64,
                            offset,
                        }))
                    )
                }
            })
            .merge_known_chunks()
            .try_for_each(move |merged_chunk_info| {

                if let MergedChunkInfo::New(chunk_info) = merged_chunk_info {
                    let offset = chunk_info.offset;
                    let digest = *chunk_info.chunk.digest();
                    let digest_str = digest_to_hex(&digest);

                    println!("upload new chunk {} ({} bytes, offset {})", digest_str,
                             chunk_info.chunk_len, offset);

                    let chunk_data = chunk_info.chunk.raw_data();
                    let param = json!({
                        "wid": wid,
                        "digest": digest_str,
                        "size": chunk_info.chunk_len,
                        "encoded-size": chunk_data.len(),
                    });

                    let request = H2Client::request_builder("localhost", "POST", &upload_chunk_path, Some(param)).unwrap();
                    let upload_data = Some(bytes::Bytes::from(chunk_data));

                    let new_info = MergedChunkInfo::Known(vec![(offset, digest)]);

                    let mut upload_queue = upload_queue.clone();
                    future::Either::Left(h2
                        .send_request(request, upload_data)
                        .and_then(move |response| async move {
                            upload_queue
                                .send((new_info, Some(response)))
                                .await
                                .map_err(Error::from)
                        })
                    )
                } else {
                    let mut upload_queue = upload_queue.clone();
                    future::Either::Right(async move {
                        upload_queue
                            .send((merged_chunk_info, None))
                            .await
                            .map_err(Error::from)
                    })
                }
            })
            .then(move |result| async move {
                upload_result.await?.and(result)
            }.boxed())
            .and_then(move |_| {
                let repeat = repeat2.load(Ordering::SeqCst);
                let stream_len = stream_len2.load(Ordering::SeqCst);
                let speed = ((stream_len*1000000)/(1024*1024))/(start_time.elapsed().as_micros() as usize);
                println!("Uploaded {} chunks in {} seconds ({} MB/s).", repeat, start_time.elapsed().as_secs(), speed);
                if repeat > 0 {
                    println!("Average chunk size was {} bytes.", stream_len/repeat);
                    println!("Time per request: {} microseconds.", (start_time.elapsed().as_micros())/(repeat as u128));
                }

                let mut guard = index_csum_2.lock().unwrap();
                let csum = guard.take().unwrap().finish();

                futures::future::ok((repeat, stream_len, speed, csum))
            })
    }

    pub fn upload_speedtest(&self) -> impl Future<Output = Result<usize, Error>> {

        let mut data = vec![];
        // generate pseudo random byte sequence
        for i in 0..1024*1024 {
            for j in 0..4 {
                let byte = ((i >> (j<<3))&0xff) as u8;
                data.push(byte);
            }
        }

        let item_len = data.len();

        let repeat = Arc::new(AtomicUsize::new(0));
        let repeat2 = repeat.clone();

        let (upload_queue, upload_result) = Self::response_queue();

        let start_time = std::time::Instant::now();

        let h2 = self.h2.clone();

        futures::stream::repeat(data)
            .take_while(move |_| {
                let repeat = Arc::clone(&repeat);
                async move {
                    repeat.fetch_add(1, Ordering::SeqCst);
                    start_time.elapsed().as_secs() < 5
                }
            })
            .map(Ok)
            .try_for_each(move |data| {
                let h2 = h2.clone();

                let mut upload_queue = upload_queue.clone();

                println!("send test data ({} bytes)", data.len());
                let request = H2Client::request_builder("localhost", "POST", "speedtest", None).unwrap();
                h2.send_request(request, Some(bytes::Bytes::from(data)))
                    .and_then(move |response| async move {
                        upload_queue
                            .send(response)
                            .await
                            .map_err(Error::from)
                    })
            })
            .then(move |result| async move {
                println!("RESULT {:?}", result);
                upload_result.await?.and(result)
            })
            .and_then(move |_| {
                let repeat = repeat2.load(Ordering::SeqCst);
                println!("Uploaded {} chunks in {} seconds.", repeat, start_time.elapsed().as_secs());
                let speed = ((item_len*1000000*(repeat as usize))/(1024*1024))/(start_time.elapsed().as_micros() as usize);
                if repeat > 0 {
                    println!("Time per request: {} microseconds.", (start_time.elapsed().as_micros())/(repeat as u128));
                }
                futures::future::ok(speed)
            })
    }
}

#[derive(Clone)]
pub struct H2Client {
    h2: h2::client::SendRequest<bytes::Bytes>,
}

impl H2Client {

    pub fn new(h2: h2::client::SendRequest<bytes::Bytes>) -> Self {
        Self { h2 }
    }

    pub fn get(&self, path: &str, param: Option<Value>) -> impl Future<Output = Result<Value, Error>> {
        let req = Self::request_builder("localhost", "GET", path, param).unwrap();
        self.request(req)
    }

    pub fn put(&self, path: &str, param: Option<Value>) -> impl Future<Output = Result<Value, Error>> {
        let req = Self::request_builder("localhost", "PUT", path, param).unwrap();
        self.request(req)
    }

    pub fn post(&self, path: &str, param: Option<Value>) -> impl Future<Output = Result<Value, Error>> {
        let req = Self::request_builder("localhost", "POST", path, param).unwrap();
        self.request(req)
    }

    pub fn download<W: Write + Send + 'static>(
        &self,
        path: &str,
        param: Option<Value>,
        output: W,
    ) -> impl Future<Output = Result<W, Error>> {
        let request = Self::request_builder("localhost", "GET", path, param).unwrap();

        self.send_request(request, None)
            .and_then(move |response| {
                response
                    .map_err(Error::from)
                    .and_then(move |resp| {
                        let status = resp.status();
                        if !status.is_success() {
                            future::Either::Left(
                                H2Client::h2api_response(resp)
                                    .map(|_| Err(format_err!("unknown error")))
                            )
                        } else {
                            let mut body = resp.into_body();
                            let release_capacity = body.release_capacity().clone();

                            future::Either::Right(
                                body
                                    .map_err(Error::from)
                                    .try_fold(output, move |mut acc, chunk| {
                                        let mut release_capacity = release_capacity.clone();
                                        async move {
                                            let _ = release_capacity.release_capacity(chunk.len());
                                            acc.write_all(&chunk)?;
                                            Ok::<_, Error>(acc)
                                        }
                                    })
                            )
                        }
                    })
            })
    }

    pub fn upload(
        &self,
        path: &str,
        param: Option<Value>,
        data: Vec<u8>,
    ) -> impl Future<Output = Result<Value, Error>> {
        let request = Self::request_builder("localhost", "POST", path, param).unwrap();

        self.h2.clone()
            .ready()
            .map_err(Error::from)
            .and_then(move |mut send_request| {
                let (response, stream) = send_request.send_request(request, false).unwrap();
                PipeToSendStream::new(bytes::Bytes::from(data), stream)
                    .and_then(|_| {
                        response
                            .map_err(Error::from)
                            .and_then(Self::h2api_response)
                    })
            })
    }

    fn request(
        &self,
        request: Request<()>,
    ) -> impl Future<Output = Result<Value, Error>> {

        self.send_request(request, None)
            .and_then(move |response| {
                response
                    .map_err(Error::from)
                    .and_then(Self::h2api_response)
            })
    }

    fn send_request(
        &self,
        request: Request<()>,
        data: Option<bytes::Bytes>,
    ) -> impl Future<Output = Result<h2::client::ResponseFuture, Error>> {

        self.h2.clone()
            .ready()
            .map_err(Error::from)
            .and_then(move |mut send_request| {
                if let Some(data) = data {
                    let (response, stream) = send_request.send_request(request, false).unwrap();
                    future::Either::Left(PipeToSendStream::new(data, stream)
                        .and_then(move |_| {
                            future::ok(response)
                        }))
                } else {
                    let (response, _stream) = send_request.send_request(request, true).unwrap();
                    future::Either::Right(future::ok(response))
                }
            })
    }

    fn h2api_response(
        response: Response<h2::RecvStream>,
    ) -> impl Future<Output = Result<Value, Error>> {
        let status = response.status();

        let (_head, mut body) = response.into_parts();

        // The `release_capacity` handle allows the caller to manage
        // flow control.
        //
        // Whenever data is received, the caller is responsible for
        // releasing capacity back to the server once it has freed
        // the data from memory.
        let mut release_capacity = body.release_capacity().clone();

        body
            .map_ok(move |chunk| {
                // Let the server send more data.
                let _ = release_capacity.release_capacity(chunk.len());
                chunk
            })
            .try_concat()
            .map_err(Error::from)
            .and_then(move |data| async move {
                let text = String::from_utf8(data.to_vec()).unwrap();
                if status.is_success() {
                    if text.len() > 0 {
                        let mut value: Value = serde_json::from_str(&text)?;
                        if let Some(map) = value.as_object_mut() {
                            if let Some(data) = map.remove("data") {
                                return Ok(data);
                            }
                        }
                        bail!("got result without data property");
                    } else {
                        Ok(Value::Null)
                    }
                } else {
                    bail!("HTTP Error {}: {}", status, text);
                }
            }.boxed())
    }

    // Note: We always encode parameters with the url
    pub fn request_builder(server: &str, method: &str, path: &str, data: Option<Value>) -> Result<Request<()>, Error> {
        let path = path.trim_matches('/');

        if let Some(data) = data {
            let query = tools::json_object_to_query(data)?;
            // We detected problem with hyper around 6000 characters - seo we try to keep on the safe side
            if query.len() > 4096 { bail!("h2 query data too large ({} bytes) - please encode data inside body", query.len()); }
            let url: Uri = format!("https://{}:8007/{}?{}", server, path, query).parse()?;
             let request = Request::builder()
                .method(method)
                .uri(url)
                .header("User-Agent", "proxmox-backup-client/1.0")
                .header(hyper::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(())?;
            return Ok(request);
        } else {
            let url: Uri = format!("https://{}:8007/{}", server, path).parse()?;
            let request = Request::builder()
                .method(method)
                .uri(url)
                .header("User-Agent", "proxmox-backup-client/1.0")
                .header(hyper::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(())?;

            Ok(request)
        }
    }
}
