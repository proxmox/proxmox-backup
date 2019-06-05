use failure::*;

use http::Uri;
use hyper::Body;
use hyper::client::Client;
use xdg::BaseDirectories;
use chrono::Utc;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use http::{Request, Response};
use http::header::HeaderValue;

use futures::*;
use futures::stream::Stream;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::mpsc;

use serde_json::{json, Value};
use url::percent_encoding::{percent_encode,  DEFAULT_ENCODE_SET};

use crate::tools::{self, BroadcastFuture, tty};
use super::pipe_to_stream::*;
use super::merge_known_chunks::*;


#[derive(Clone)]
struct AuthInfo {
    username: String,
    ticket: String,
    token: String,
}

/// HTTP(S) API client
pub struct HttpClient {
    client: Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>,
    server: String,
    auth: BroadcastFuture<AuthInfo>,
}

fn store_ticket_info(server: &str, username: &str, ticket: &str, token: &str) -> Result<(), Error> {

    let base = BaseDirectories::with_prefix("proxmox-backup")?;

    // usually /run/user/<uid>/...
    let path = base.place_runtime_file("tickets")?;

    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);

    let mut data = tools::file_get_json(&path, Some(json!({})))?;

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

    tools::file_set_contents(path, new_data.to_string().as_bytes(), Some(mode))?;

    Ok(())
}

fn load_ticket_info(server: &str, username: &str) -> Option<(String, String)> {
    let base = match BaseDirectories::with_prefix("proxmox-backup") {
        Ok(b) => b,
        _ => return None,
    };

    // usually /run/user/<uid>/...
    let path = match base.place_runtime_file("tickets") {
        Ok(p) => p,
        _ => return None,
    };

    let data = match tools::file_get_json(&path, None) {
        Ok(v) => v,
        _ => return None,
    };

    let now = Utc::now().timestamp();

    let ticket_lifetime = tools::ticket::TICKET_LIFETIME - 60;

    if let Some(uinfo) = data[server][username].as_object() {
        if let Some(timestamp) = uinfo["timestamp"].as_i64() {
            let age = now - timestamp;
            if age < ticket_lifetime {
                let ticket = match uinfo["ticket"].as_str() {
                    Some(t) => t,
                    None => return None,
                };
                let token = match uinfo["token"].as_str() {
                    Some(t) => t,
                    None => return None,
                };
                return Some((ticket.to_owned(), token.to_owned()));
            }
        }
    }

    None
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

    fn build_client() -> Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>> {
        let mut builder = native_tls::TlsConnector::builder();
        // FIXME: We need a CLI option for this!
        builder.danger_accept_invalid_certs(true);
        let tlsconnector = builder.build().unwrap();
        let mut httpc = hyper::client::HttpConnector::new(1);
        //httpc.set_nodelay(true); // not sure if this help?
        httpc.enforce_http(false); // we want https...
        let mut https = hyper_tls::HttpsConnector::from((httpc, tlsconnector));
        https.https_only(true); // force it!
        Client::builder()
        //.http2_initial_stream_window_size( (1 << 31) - 2)
        //.http2_initial_connection_window_size( (1 << 31) - 2)
            .build::<_, Body>(https)
    }

    pub fn request(&self, mut req: Request<Body>) -> impl Future<Item=Value, Error=Error>  {

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

    pub fn get(&self, path: &str, data: Option<Value>) -> impl Future<Item=Value, Error=Error> {

        let req = Self::request_builder(&self.server, "GET", path, data).unwrap();
        self.request(req)
    }

    pub fn delete(&mut self, path: &str, data: Option<Value>) -> impl Future<Item=Value, Error=Error> {

        let req = Self::request_builder(&self.server, "DELETE", path, data).unwrap();
        self.request(req)
    }

    pub fn post(&mut self, path: &str, data: Option<Value>) -> impl Future<Item=Value, Error=Error> {

        let req = Self::request_builder(&self.server, "POST", path, data).unwrap();
        self.request(req)
    }

    pub fn download(&mut self, path: &str, mut output: Box<dyn std::io::Write + Send>) ->  impl Future<Item=(), Error=Error> {

        let mut req = Self::request_builder(&self.server, "GET", path, None).unwrap();

        let login = self.auth.listen();

        let client = self.client.clone();

        login.and_then(move |auth| {

            let enc_ticket = format!("PBSAuthCookie={}", percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET));
            req.headers_mut().insert("Cookie", HeaderValue::from_str(&enc_ticket).unwrap());

            client.request(req)
                .map_err(Error::from)
                .and_then(|resp| {

                    let _status = resp.status(); // fixme: ??

                    resp.into_body()
                        .map_err(Error::from)
                        .for_each(move |chunk| {
                            output.write_all(&chunk)?;
                            Ok(())
                        })

                })
        })
    }

    pub fn upload(&mut self, content_type: &str, body: Body, path: &str) -> impl Future<Item=Value, Error=Error> {

        let path = path.trim_matches('/');
        let url: Uri = format!("https://{}:8007/{}", &self.server, path).parse().unwrap();

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
        debug: bool,
    ) -> impl Future<Item=BackupClient, Error=Error> {

        let param = json!({"backup-type": backup_type, "backup-id": backup_id, "store": datastore, "debug": debug});
        let mut req = Self::request_builder(&self.server, "GET", "/api2/json/backup", Some(param)).unwrap();

        let login = self.auth.listen();

        let client = self.client.clone();

        login.and_then(move |auth| {

            let enc_ticket = format!("PBSAuthCookie={}", percent_encode(auth.ticket.as_bytes(), DEFAULT_ENCODE_SET));
            req.headers_mut().insert("Cookie", HeaderValue::from_str(&enc_ticket).unwrap());
            req.headers_mut().insert("UPGRADE", HeaderValue::from_str(crate::backup::PROXMOX_BACKUP_PROTOCOL_ID_V1).unwrap());

            client.request(req)
                .map_err(Error::from)
                .and_then(|resp| {

                    let status = resp.status();
                    if status != http::StatusCode::SWITCHING_PROTOCOLS {
                        future::Either::A(Self::api_response(resp).and_then(|_| { bail!("unknown error"); }))
                    } else {
                        future::Either::B(resp.into_body().on_upgrade().map_err(Error::from))
                    }
                })
                .and_then(|upgraded| {
                    h2::client::handshake(upgraded).map_err(Error::from)
                })
                .and_then(|(h2, connection)| {
                    let connection = connection
                        .map_err(|_| panic!("HTTP/2.0 connection failed"));

                    // Spawn a new task to drive the connection state
                    hyper::rt::spawn(connection);

                    // Wait until the `SendRequest` handle has available capacity.
                    h2.ready()
                        .map(BackupClient::new)
                        .map_err(Error::from)
                })
        })
    }

    fn credentials(
        client: Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>,
        server: String,
        username: String,
        password: String,
    ) -> Box<Future<Item=AuthInfo, Error=Error> + Send> {

        let server2 = server.clone();

        let create_request = futures::future::lazy(move || {
            let data = json!({ "username": username, "password": password });
            let req = Self::request_builder(&server, "POST", "/api2/json/access/ticket", Some(data)).unwrap();
            Self::api_request(client, req)
        });

        let login_future = create_request
            .and_then(move |cred| {
                let auth = AuthInfo {
                    username: cred["data"]["username"].as_str().unwrap().to_owned(),
                    ticket: cred["data"]["ticket"].as_str().unwrap().to_owned(),
                    token: cred["data"]["CSRFPreventionToken"].as_str().unwrap().to_owned(),
                };

                let _ = store_ticket_info(&server2, &auth.username, &auth.ticket, &auth.token);

                Ok(auth)
            });

        Box::new(login_future)
    }

    fn api_response(response: Response<Body>) -> impl Future<Item=Value, Error=Error> {

        let status = response.status();

        response
            .into_body()
            .concat2()
            .map_err(Error::from)
            .and_then(move |data| {

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
            })
    }

    fn api_request(
        client: Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>>,
        req: Request<Body>
    ) -> impl Future<Item=Value, Error=Error> {

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

//#[derive(Clone)]
pub struct BackupClient {
    h2: H2Client,
}


impl BackupClient {

    pub fn new(h2: h2::client::SendRequest<bytes::Bytes>) -> Self {
        Self { h2: H2Client::new(h2) }
    }

    pub fn get(&self, path: &str, param: Option<Value>) -> impl Future<Item=Value, Error=Error> {
        self.h2.get(path, param)
    }

    pub fn put(&self, path: &str, param: Option<Value>) -> impl Future<Item=Value, Error=Error> {
        self.h2.put(path, param)
    }

    pub fn post(&self, path: &str, param: Option<Value>) -> impl Future<Item=Value, Error=Error> {
        self.h2.post(path, param)
    }

    pub fn finish(&self) -> impl Future<Item=(), Error=Error> {
        self.h2.clone().post("finish", None).map(|_| ())
    }

    pub fn upload_config<P: AsRef<std::path::Path>>(
        &self,
        src_path: P,
        file_name: &str,
     ) -> impl Future<Item=(), Error=Error> {

        let h2 = self.h2.clone();
        let file_name = file_name.to_owned();
        let src_path = src_path.as_ref().to_owned();

        let task = tokio::fs::File::open(src_path.clone())
            .map_err(move |err| format_err!("unable to open file {:?} - {}", src_path, err))
            .and_then(|file| {
                let contents = vec![];
                tokio::io::read_to_end(file, contents)
                    .map_err(Error::from)
                    .and_then(move |(_, contents)| {
                        let param = json!({"size": contents.len(), "file-name": file_name });
                        h2.upload("config", Some(param), contents)
                            .map(|_| {})
                    })
            });

        task
    }

    pub fn upload_stream(
        &self,
        archive_name: &str,
        stream: impl Stream<Item=bytes::BytesMut, Error=Error>,
        prefix: &str,
        fixed_size: Option<u64>,
    ) -> impl Future<Item=(), Error=Error> {

        let known_chunks = Arc::new(Mutex::new(HashSet::new()));

        let mut stream_len = 0u64;

        let stream = stream.
            map(move |data| {
                let digest = openssl::sha::sha256(&data);
                let offset = stream_len;
                stream_len += data.len() as u64;
                ChunkInfo { data, digest, offset }
            });

        let h2 = self.h2.clone();
        let h2_2 = self.h2.clone();
        let h2_3 = self.h2.clone();
        let h2_4 = self.h2.clone();

        let mut param = json!({ "archive-name": archive_name });
        if let Some(size) = fixed_size {
            param["size"] = size.into();
        }

        let index_path = format!("{}_index", prefix);
        let close_path = format!("{}_close", prefix);

        let prefix = prefix.to_owned();

        Self::download_chunk_list(h2, &index_path, archive_name, known_chunks.clone())
            .and_then(move |_| {
                h2_2.post(&index_path, Some(param))
            })
            .and_then(move |res| {
                let wid = res.as_u64().unwrap();
                Self::upload_chunk_info_stream(h2_3, wid, stream, &prefix, known_chunks.clone())
                     .and_then(move |(chunk_count, size, _speed)| {
                        let param = json!({
                            "wid": wid ,
                            "chunk-count": chunk_count,
                            "size": size,
                        });
                        h2_4.post(&close_path, Some(param))
                     })
                    .map(|_| ())
            })
    }

    fn response_queue() -> (
        mpsc::Sender<h2::client::ResponseFuture>,
        sync::oneshot::Receiver<Result<(), Error>>
    ) {
        let (verify_queue_tx, verify_queue_rx) = mpsc::channel(100);
        let (verify_result_tx, verify_result_rx) = sync::oneshot::channel();

        hyper::rt::spawn(
            verify_queue_rx
                .map_err(Error::from)
                .for_each(|response: h2::client::ResponseFuture| {
                    response
                        .map_err(Error::from)
                        .and_then(H2Client::h2api_response)
                        .and_then(|result| {
                            println!("RESPONSE: {:?}", result);
                            Ok(())
                        })
                        .map_err(|err| format_err!("pipelined request failed: {}", err))
                })
                .then(|result|
                      verify_result_tx.send(result)
                )
                .map_err(|_| { /* ignore closed channel */ })
        );

        (verify_queue_tx, verify_result_rx)
    }

    fn append_chunk_queue(h2: H2Client, wid: u64, path: String) -> (
        mpsc::Sender<(MergedChunkInfo, Option<h2::client::ResponseFuture>)>,
        sync::oneshot::Receiver<Result<(), Error>>
    ) {
        let (verify_queue_tx, verify_queue_rx) = mpsc::channel(64);
        let (verify_result_tx, verify_result_rx) = sync::oneshot::channel();

        let h2_2 = h2.clone();

        hyper::rt::spawn(
            verify_queue_rx
                .map_err(Error::from)
                .and_then(move |(merged_chunk_info, response): (MergedChunkInfo, Option<h2::client::ResponseFuture>)| {
                    match (response, merged_chunk_info) {
                        (Some(response), MergedChunkInfo::Known(list)) => {
                            future::Either::A(
                                response
                                    .map_err(Error::from)
                                    .and_then(H2Client::h2api_response)
                                    .and_then(move |_result| {
                                        Ok(MergedChunkInfo::Known(list))
                                    })
                            )
                        }
                        (None, MergedChunkInfo::Known(list)) => {
                            future::Either::B(future::ok(MergedChunkInfo::Known(list)))
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
                                //println!("append chunk {} (offset {})", tools::digest_to_hex(&digest), offset);
                                digest_list.push(tools::digest_to_hex(&digest));
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
                                        .and_then(|_| Ok(()))
                                })
                                .map_err(|err| format_err!("pipelined request failed: {}", err))
                        }
                        _ => unreachable!(),
                    }
                })
                .for_each(|_| Ok(()))
                .then(|result|
                      verify_result_tx.send(result)
                )
                .map_err(|_| { /* ignore closed channel */ })
        );

        (verify_queue_tx, verify_result_rx)
    }

    fn download_chunk_list(
        h2: H2Client,
        path: &str,
        archive_name: &str,
        known_chunks: Arc<Mutex<HashSet<[u8;32]>>>,
    ) -> impl Future<Item=(), Error=Error> {

        let param = json!({ "archive-name": archive_name });
        let request = H2Client::request_builder("localhost", "GET", path, Some(param)).unwrap();

        h2.send_request(request, None)
            .and_then(move |response| {
                response
                    .map_err(Error::from)
                    .and_then(move |resp| {
                        let status = resp.status();

                        if !status.is_success() {
                            future::Either::A(H2Client::h2api_response(resp).and_then(|_| { bail!("unknown error"); }))
                        } else {
                            future::Either::B(future::ok(resp.into_body()))
                        }
                    })
                    .and_then(move |mut body| {

                        let mut release_capacity = body.release_capacity().clone();

                        crate::backup::DigestListDecoder::new(body.map_err(Error::from))
                            .for_each(move |chunk| {
                                let _ = release_capacity.release_capacity(chunk.len());
                                println!("GOT DOWNLOAD {}", tools::digest_to_hex(&chunk));
                                known_chunks.lock().unwrap().insert(chunk);
                                Ok(())
                            })
                       })
            })
    }

    fn upload_chunk_info_stream(
        h2: H2Client,
        wid: u64,
        stream: impl Stream<Item=ChunkInfo, Error=Error>,
        prefix: &str,
        known_chunks: Arc<Mutex<HashSet<[u8;32]>>>,
    ) -> impl Future<Item=(usize, usize, usize), Error=Error> {

        let repeat = std::sync::Arc::new(AtomicUsize::new(0));
        let repeat2 = repeat.clone();

        let stream_len = std::sync::Arc::new(AtomicUsize::new(0));
        let stream_len2 = stream_len.clone();

        let append_chunk_path = format!("{}_index", prefix);
        let upload_chunk_path = format!("{}_chunk", prefix);

        let (upload_queue, upload_result) = Self::append_chunk_queue(h2.clone(), wid, append_chunk_path.to_owned());

        let start_time = std::time::Instant::now();

        stream
            .map(move |chunk_info| {
                repeat.fetch_add(1, Ordering::SeqCst);
                stream_len.fetch_add(chunk_info.data.len(), Ordering::SeqCst);

                let mut known_chunks = known_chunks.lock().unwrap();
                let chunk_is_known = known_chunks.contains(&chunk_info.digest);
                if chunk_is_known {
                    MergedChunkInfo::Known(vec![(chunk_info.offset, chunk_info.digest)])
                } else {
                    known_chunks.insert(chunk_info.digest);
                    MergedChunkInfo::New(chunk_info)
                }
            })
            .merge_known_chunks()
            .for_each(move |merged_chunk_info| {

                if let MergedChunkInfo::New(chunk_info) = merged_chunk_info {
                    let offset = chunk_info.offset;
                    let digest = chunk_info.digest;
                    let upload_queue = upload_queue.clone();

                    println!("upload new chunk {} ({} bytes, offset {})", tools::digest_to_hex(&digest),
                             chunk_info.data.len(), offset);

                    let param = json!({ "wid": wid, "size" : chunk_info.data.len() });
                    let request = H2Client::request_builder("localhost", "POST", &upload_chunk_path, Some(param)).unwrap();
                    let upload_data = Some(chunk_info.data.freeze());

                    let new_info = MergedChunkInfo::Known(vec![(offset, digest)]);

                    future::Either::A(
                        h2.send_request(request, upload_data)
                            .and_then(move |response| {
                                upload_queue.clone().send((new_info, Some(response)))
                                    .map(|_| ()).map_err(Error::from)
                            })
                    )
                } else {

                    future::Either::B(
                        upload_queue.clone().send((merged_chunk_info, None))
                            .map(|_| ()).map_err(Error::from)
                    )
                }
            })
            .then(move |result| {
                println!("RESULT {:?}", result);
                upload_result.map_err(Error::from).and_then(|upload1_result| {
                    Ok(upload1_result.and(result))
                })
            })
            .flatten()
            .and_then(move |_| {
                let repeat = repeat2.load(Ordering::SeqCst);
                let stream_len = stream_len2.load(Ordering::SeqCst);
                let speed = ((stream_len*1000000)/(1024*1024))/(start_time.elapsed().as_micros() as usize);
                println!("Uploaded {} chunks in {} seconds ({} MB/s).", repeat, start_time.elapsed().as_secs(), speed);
                if repeat > 0 {
                    println!("Average chunk size was {} bytes.", stream_len/repeat);
                    println!("Time per request: {} microseconds.", (start_time.elapsed().as_micros())/(repeat as u128));
                }
                Ok((repeat, stream_len, speed))
            })
    }

    pub fn upload_speedtest(&self) -> impl Future<Item=usize, Error=Error> {

        let mut data = vec![];
        // generate pseudo random byte sequence
        for i in 0..1024*1024 {
            for j in 0..4 {
                let byte = ((i >> (j<<3))&0xff) as u8;
                data.push(byte);
            }
        }

        let item_len = data.len();

        let repeat = std::sync::Arc::new(AtomicUsize::new(0));
        let repeat2 = repeat.clone();

        let (upload_queue, upload_result) = Self::response_queue();

        let start_time = std::time::Instant::now();

        let h2 = self.h2.clone();

        futures::stream::repeat(data)
            .take_while(move |_| {
                repeat.fetch_add(1, Ordering::SeqCst);
                Ok(start_time.elapsed().as_secs() < 5)
            })
            .for_each(move |data| {
                let h2 = h2.clone();

                let upload_queue = upload_queue.clone();

                println!("send test data ({} bytes)", data.len());
                let request = H2Client::request_builder("localhost", "POST", "speedtest", None).unwrap();
                h2.send_request(request, Some(bytes::Bytes::from(data)))
                    .and_then(move |response| {
                        upload_queue.send(response)
                            .map(|_| ()).map_err(Error::from)
                    })
            })
            .then(move |result| {
                println!("RESULT {:?}", result);
                upload_result.map_err(Error::from).and_then(|upload1_result| {
                    Ok(upload1_result.and(result))
                })
            })
            .flatten()
            .and_then(move |_| {
                let repeat = repeat2.load(Ordering::SeqCst);
                println!("Uploaded {} chunks in {} seconds.", repeat, start_time.elapsed().as_secs());
                let speed = ((item_len*1000000*(repeat as usize))/(1024*1024))/(start_time.elapsed().as_micros() as usize);
                if repeat > 0 {
                    println!("Time per request: {} microseconds.", (start_time.elapsed().as_micros())/(repeat as u128));
                }
                Ok(speed)
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

    pub fn get(&self, path: &str, param: Option<Value>) -> impl Future<Item=Value, Error=Error> {
        let req = Self::request_builder("localhost", "GET", path, param).unwrap();
        self.request(req)
    }

    pub fn put(&self, path: &str, param: Option<Value>) -> impl Future<Item=Value, Error=Error> {
        let req = Self::request_builder("localhost", "PUT", path, param).unwrap();
        self.request(req)
    }

    pub fn post(&self, path: &str, param: Option<Value>) -> impl Future<Item=Value, Error=Error> {
        let req = Self::request_builder("localhost", "POST", path, param).unwrap();
        self.request(req)
    }

    pub fn upload(&self, path: &str, param: Option<Value>, data: Vec<u8>) -> impl Future<Item=Value, Error=Error> {
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
    ) -> impl Future<Item=Value, Error=Error> {

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
    ) -> impl Future<Item=h2::client::ResponseFuture, Error=Error> {

        self.h2.clone()
            .ready()
            .map_err(Error::from)
            .and_then(move |mut send_request| {
                if let Some(data) = data {
                    let (response, stream) = send_request.send_request(request, false).unwrap();
                    future::Either::A(PipeToSendStream::new(bytes::Bytes::from(data), stream)
                        .and_then(move |_| {
                            future::ok(response)
                        }))
                } else {
                    let (response, _stream) = send_request.send_request(request, true).unwrap();
                    future::Either::B(future::ok(response))
                }
            })
    }

    fn h2api_response(response: Response<h2::RecvStream>) -> impl Future<Item=Value, Error=Error> {

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
            .map(move |chunk| {
                // Let the server send more data.
                let _ = release_capacity.release_capacity(chunk.len());
                chunk
            })
            .concat2()
            .map_err(Error::from)
            .and_then(move |data| {
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
            })
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
