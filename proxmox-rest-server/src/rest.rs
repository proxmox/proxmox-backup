use std::collections::HashMap;
use std::future::Future;
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use anyhow::{bail, format_err, Error};
use futures::future::{self, FutureExt, TryFutureExt};
use futures::stream::TryStreamExt;
use hyper::body::HttpBody;
use hyper::header::{self, HeaderMap};
use hyper::http::request::Parts;
use hyper::{Body, Request, Response, StatusCode};
use lazy_static::lazy_static;
use regex::Regex;
use serde_json::Value;
use tokio::fs::File;
use tokio::time::Instant;
use url::form_urlencoded;
use tower_service::Service;

use proxmox_router::{
    check_api_permission, ApiHandler, ApiMethod, HttpError, Permission, RpcEnvironment,
    RpcEnvironmentType, UserInformation,
};
use proxmox_router::http_err;
use proxmox_schema::{
    parse_parameter_strings, parse_simple_value, verify_json_object, ObjectSchemaType,
    ParameterSchema,
};

use pbs_tools::compression::{DeflateEncoder, Level};
use pbs_tools::stream::AsyncReaderStream;

use crate::{
    ApiConfig, FileLogger, AuthError, RestEnvironment, CompressionMethod,
    normalize_uri_path, formatter::*,
};

extern "C" {
    fn tzset();
}

struct AuthStringExtension(String);

struct EmptyUserInformation {}

impl UserInformation for EmptyUserInformation {
    fn is_superuser(&self, _userid: &str) -> bool { false }
    fn is_group_member(&self, _userid: &str, _group: &str) -> bool { false }
    fn lookup_privs(&self, _userid: &str, _path: &[&str]) -> u64 { 0 }
}

/// REST server implementation (configured with [ApiConfig])
///
/// This struct implements the [Service] trait in order to use it with
/// [hyper::server::Builder::serve].
pub struct RestServer {
    api_config: Arc<ApiConfig>,
}

const MAX_URI_QUERY_LENGTH: usize = 3072;
const CHUNK_SIZE_LIMIT: u64 = 32 * 1024;

impl RestServer {
    /// Creates a new instance.
    pub fn new(api_config: ApiConfig) -> Self {
        Self {
            api_config: Arc::new(api_config),
        }
    }
}

impl Service<&Pin<Box<tokio_openssl::SslStream<tokio::net::TcpStream>>>>
    for RestServer
{
    type Response = ApiService;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<ApiService, Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(
        &mut self,
        ctx: &Pin<Box<tokio_openssl::SslStream<tokio::net::TcpStream>>>,
    ) -> Self::Future {
        match ctx.get_ref().peer_addr() {
            Err(err) => future::err(format_err!("unable to get peer address - {}", err)).boxed(),
            Ok(peer) => future::ok(ApiService {
                peer,
                api_config: self.api_config.clone(),
            })
            .boxed(),
        }
    }
}

impl Service<&hyper::server::conn::AddrStream> for RestServer {
    type Response = ApiService;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<ApiService, Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, ctx: &hyper::server::conn::AddrStream) -> Self::Future {
        let peer =  ctx.remote_addr();
        future::ok(ApiService {
            peer,
            api_config: self.api_config.clone(),
        })
        .boxed()
    }
}

impl Service<&tokio::net::UnixStream> for RestServer {
    type Response = ApiService;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<ApiService, Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _ctx: &tokio::net::UnixStream) -> Self::Future {
        // TODO: Find a way to actually represent the vsock peer in the ApiService struct - for now
        // it doesn't really matter, so just use a fake IP address
        let fake_peer = "0.0.0.0:807".parse().unwrap();
        future::ok(ApiService {
            peer: fake_peer,
            api_config: self.api_config.clone(),
        })
        .boxed()
    }
}

// Helper [Service] containing the peer Address
//
// The lower level connection [Service] implementation on
// [RestServer] extracts the peer address and return an [ApiService].
//
// Rust wants this type 'pub' here (else we get 'private type `ApiService`
// in public interface'). The type is still private because the crate does
// not export it.
pub struct ApiService {
    pub peer: std::net::SocketAddr,
    pub api_config: Arc<ApiConfig>,
}

fn log_response(
    logfile: Option<&Arc<Mutex<FileLogger>>>,
    peer: &std::net::SocketAddr,
    method: hyper::Method,
    path_query: &str,
    resp: &Response<Body>,
    user_agent: Option<String>,
) {
    if resp.extensions().get::<NoLogExtension>().is_some() {
        return;
    };

    // we also log URL-to-long requests, so avoid message bigger than PIPE_BUF (4k on Linux)
    // to profit from atomicty guarantees for O_APPEND opened logfiles
    let path = &path_query[..MAX_URI_QUERY_LENGTH.min(path_query.len())];

    let status = resp.status();
    if !(status.is_success() || status.is_informational()) {
        let reason = status.canonical_reason().unwrap_or("unknown reason");

        let message = match resp.extensions().get::<ErrorMessageExtension>() {
            Some(data) => &data.0,
            None => "request failed",
        };

        log::error!(
            "{} {}: {} {}: [client {}] {}",
            method.as_str(),
            path,
            status.as_str(),
            reason,
            peer,
            message
        );
    }
    if let Some(logfile) = logfile {
        let auth_id = match resp.extensions().get::<AuthStringExtension>() {
            Some(AuthStringExtension(auth_id)) => auth_id.clone(),
            None => "-".to_string(),
        };
        let now = proxmox_time::epoch_i64();
        // time format which apache/nginx use (by default), copied from pve-http-server
        let datetime = proxmox_time::strftime_local("%d/%m/%Y:%H:%M:%S %z", now)
            .unwrap_or_else(|_| "-".to_string());

        logfile.lock().unwrap().log(format!(
            "{} - {} [{}] \"{} {}\" {} {} {}",
            peer.ip(),
            auth_id,
            datetime,
            method.as_str(),
            path,
            status.as_str(),
            resp.body().size_hint().lower(),
            user_agent.unwrap_or_else(|| "-".to_string()),
        ));
    }
}

fn get_proxied_peer(headers: &HeaderMap) -> Option<std::net::SocketAddr> {
    lazy_static! {
        static ref RE: Regex = Regex::new(r#"for="([^"]+)""#).unwrap();
    }
    let forwarded = headers.get(header::FORWARDED)?.to_str().ok()?;
    let capture = RE.captures(&forwarded)?;
    let rhost = capture.get(1)?.as_str();

    rhost.parse().ok()
}

fn get_user_agent(headers: &HeaderMap) -> Option<String> {
    let agent = headers.get(header::USER_AGENT)?.to_str();
    agent
        .map(|s| {
            let mut s = s.to_owned();
            s.truncate(128);
            s
        })
        .ok()
}

impl Service<Request<Body>> for ApiService {
    type Response = Response<Body>;
    type Error = Error;
    #[allow(clippy::type_complexity)]
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let path = req.uri().path_and_query().unwrap().as_str().to_owned();
        let method = req.method().clone();
        let user_agent = get_user_agent(req.headers());

        let config = Arc::clone(&self.api_config);
        let peer = match get_proxied_peer(req.headers()) {
            Some(proxied_peer) => proxied_peer,
            None => self.peer,
        };
        async move {
            let response = match handle_request(Arc::clone(&config), req, &peer).await {
                Ok(response) => response,
                Err(err) => {
                    let (err, code) = match err.downcast_ref::<HttpError>() {
                        Some(apierr) => (apierr.message.clone(), apierr.code),
                        _ => (err.to_string(), StatusCode::BAD_REQUEST),
                    };
                    Response::builder()
                        .status(code)
                        .extension(ErrorMessageExtension(err.to_string()))
                        .body(err.into())?
                }
            };
            let logger = config.get_access_log();
            log_response(logger, &peer, method, &path, &response, user_agent);
            Ok(response)
        }
        .boxed()
    }
}

fn parse_query_parameters<S: 'static + BuildHasher + Send>(
    param_schema: ParameterSchema,
    form: &str, // x-www-form-urlencoded body data
    parts: &Parts,
    uri_param: &HashMap<String, String, S>,
) -> Result<Value, Error> {
    let mut param_list: Vec<(String, String)> = vec![];

    if !form.is_empty() {
        for (k, v) in form_urlencoded::parse(form.as_bytes()).into_owned() {
            param_list.push((k, v));
        }
    }

    if let Some(query_str) = parts.uri.query() {
        for (k, v) in form_urlencoded::parse(query_str.as_bytes()).into_owned() {
            if k == "_dc" {
                continue;
            } // skip extjs "disable cache" parameter
            param_list.push((k, v));
        }
    }

    for (k, v) in uri_param {
        param_list.push((k.clone(), v.clone()));
    }

    let params = parse_parameter_strings(&param_list, param_schema, true)?;

    Ok(params)
}

async fn get_request_parameters<S: 'static + BuildHasher + Send>(
    param_schema: ParameterSchema,
    parts: Parts,
    req_body: Body,
    uri_param: HashMap<String, String, S>,
) -> Result<Value, Error> {
    let mut is_json = false;

    if let Some(value) = parts.headers.get(header::CONTENT_TYPE) {
        match value.to_str().map(|v| v.split(';').next()) {
            Ok(Some("application/x-www-form-urlencoded")) => {
                is_json = false;
            }
            Ok(Some("application/json")) => {
                is_json = true;
            }
            _ => bail!("unsupported content type {:?}", value.to_str()),
        }
    }

    let body = TryStreamExt::map_err(req_body, |err| {
        http_err!(BAD_REQUEST, "Problems reading request body: {}", err)
    })
    .try_fold(Vec::new(), |mut acc, chunk| async move {
        // FIXME: max request body size?
        if acc.len() + chunk.len() < 64 * 1024 {
            acc.extend_from_slice(&*chunk);
            Ok(acc)
        } else {
            Err(http_err!(BAD_REQUEST, "Request body too large"))
        }
    })
    .await?;

    let utf8_data =
        std::str::from_utf8(&body).map_err(|err| format_err!("Request body not uft8: {}", err))?;

    if is_json {
        let mut params: Value = serde_json::from_str(utf8_data)?;
        for (k, v) in uri_param {
            if let Some((_optional, prop_schema)) = param_schema.lookup(&k) {
                params[&k] = parse_simple_value(&v, prop_schema)?;
            }
        }
        verify_json_object(&params, &param_schema)?;
        return Ok(params);
    } else {
        parse_query_parameters(param_schema, utf8_data, &parts, &uri_param)
    }
}

struct NoLogExtension();

async fn proxy_protected_request(
    info: &'static ApiMethod,
    mut parts: Parts,
    req_body: Body,
    peer: &std::net::SocketAddr,
) -> Result<Response<Body>, Error> {
    let mut uri_parts = parts.uri.clone().into_parts();

    uri_parts.scheme = Some(http::uri::Scheme::HTTP);
    uri_parts.authority = Some(http::uri::Authority::from_static("127.0.0.1:82"));
    let new_uri = http::Uri::from_parts(uri_parts).unwrap();

    parts.uri = new_uri;

    let mut request = Request::from_parts(parts, req_body);
    request.headers_mut().insert(
        header::FORWARDED,
        format!("for=\"{}\";", peer).parse().unwrap(),
    );

    let reload_timezone = info.reload_timezone;

    let resp = hyper::client::Client::new()
        .request(request)
        .map_err(Error::from)
        .map_ok(|mut resp| {
            resp.extensions_mut().insert(NoLogExtension());
            resp
        })
        .await?;

    if reload_timezone {
        unsafe {
            tzset();
        }
    }

    Ok(resp)
}

pub(crate) async fn handle_api_request<Env: RpcEnvironment, S: 'static + BuildHasher + Send>(
    mut rpcenv: Env,
    info: &'static ApiMethod,
    formatter: &'static dyn OutputFormatter,
    parts: Parts,
    req_body: Body,
    uri_param: HashMap<String, String, S>,
) -> Result<Response<Body>, Error> {
    let delay_unauth_time = std::time::Instant::now() + std::time::Duration::from_millis(3000);
    let compression = extract_compression_method(&parts.headers);

    let result = match info.handler {
        ApiHandler::AsyncHttp(handler) => {
            let params = parse_query_parameters(info.parameters, "", &parts, &uri_param)?;
            (handler)(parts, req_body, params, info, Box::new(rpcenv)).await
        }
        ApiHandler::Sync(handler) => {
            let params =
                get_request_parameters(info.parameters, parts, req_body, uri_param).await?;
            (handler)(params, info, &mut rpcenv).map(|data| formatter.format_data(data, &rpcenv))
        }
        ApiHandler::Async(handler) => {
            let params =
                get_request_parameters(info.parameters, parts, req_body, uri_param).await?;
            (handler)(params, info, &mut rpcenv)
                .await
                .map(|data| formatter.format_data(data, &rpcenv))
        }
    };

    let mut resp = match result {
        Ok(resp) => resp,
        Err(err) => {
            if let Some(httperr) = err.downcast_ref::<HttpError>() {
                if httperr.code == StatusCode::UNAUTHORIZED {
                    tokio::time::sleep_until(Instant::from_std(delay_unauth_time)).await;
                }
            }
            formatter.format_error(err)
        }
    };

    let resp = match compression {
        Some(CompressionMethod::Deflate) => {
            resp.headers_mut().insert(
                header::CONTENT_ENCODING,
                CompressionMethod::Deflate.content_encoding(),
            );
            resp.map(|body| {
                Body::wrap_stream(DeflateEncoder::with_quality(
                    TryStreamExt::map_err(body, |err| {
                        proxmox::io_format_err!("error during compression: {}", err)
                    }),
                    Level::Default,
                ))
            })
        }
        None => resp,
    };

    if info.reload_timezone {
        unsafe {
            tzset();
        }
    }

    Ok(resp)
}


fn extension_to_content_type(filename: &Path) -> (&'static str, bool) {
    if let Some(ext) = filename.extension().and_then(|osstr| osstr.to_str()) {
        return match ext {
            "css" => ("text/css", false),
            "html" => ("text/html", false),
            "js" => ("application/javascript", false),
            "json" => ("application/json", false),
            "map" => ("application/json", false),
            "png" => ("image/png", true),
            "ico" => ("image/x-icon", true),
            "gif" => ("image/gif", true),
            "svg" => ("image/svg+xml", false),
            "jar" => ("application/java-archive", true),
            "woff" => ("application/font-woff", true),
            "woff2" => ("application/font-woff2", true),
            "ttf" => ("application/font-snft", true),
            "pdf" => ("application/pdf", true),
            "epub" => ("application/epub+zip", true),
            "mp3" => ("audio/mpeg", true),
            "oga" => ("audio/ogg", true),
            "tgz" => ("application/x-compressed-tar", true),
            _ => ("application/octet-stream", false),
        };
    }

    ("application/octet-stream", false)
}

async fn simple_static_file_download(
    filename: PathBuf,
    content_type: &'static str,
    compression: Option<CompressionMethod>,
) -> Result<Response<Body>, Error> {
    use tokio::io::AsyncReadExt;

    let mut file = File::open(filename)
        .await
        .map_err(|err| http_err!(BAD_REQUEST, "File open failed: {}", err))?;

    let mut data: Vec<u8> = Vec::new();

    let mut response = match compression {
        Some(CompressionMethod::Deflate) => {
            let mut enc = DeflateEncoder::with_quality(data, Level::Default);
            enc.compress_vec(&mut file, CHUNK_SIZE_LIMIT as usize)
                .await?;
            let mut response = Response::new(enc.into_inner().into());
            response.headers_mut().insert(
                header::CONTENT_ENCODING,
                CompressionMethod::Deflate.content_encoding(),
            );
            response
        }
        None => {
            file.read_to_end(&mut data)
                .await
                .map_err(|err| http_err!(BAD_REQUEST, "File read failed: {}", err))?;
            Response::new(data.into())
        }
    };

    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(content_type),
    );

    Ok(response)
}

async fn chuncked_static_file_download(
    filename: PathBuf,
    content_type: &'static str,
    compression: Option<CompressionMethod>,
) -> Result<Response<Body>, Error> {
    let mut resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type);

    let file = File::open(filename)
        .await
        .map_err(|err| http_err!(BAD_REQUEST, "File open failed: {}", err))?;

    let body = match compression {
        Some(CompressionMethod::Deflate) => {
            resp = resp.header(
                header::CONTENT_ENCODING,
                CompressionMethod::Deflate.content_encoding(),
            );
            Body::wrap_stream(DeflateEncoder::with_quality(
                AsyncReaderStream::new(file),
                Level::Default,
            ))
        }
        None => Body::wrap_stream(AsyncReaderStream::new(file)),
    };

    Ok(resp.body(body).unwrap())
}

async fn handle_static_file_download(
    filename: PathBuf,
    compression: Option<CompressionMethod>,
) -> Result<Response<Body>, Error> {
    let metadata = tokio::fs::metadata(filename.clone())
        .map_err(|err| http_err!(BAD_REQUEST, "File access problems: {}", err))
        .await?;

    let (content_type, nocomp) = extension_to_content_type(&filename);
    let compression = if nocomp { None } else { compression };

    if metadata.len() < CHUNK_SIZE_LIMIT {
        simple_static_file_download(filename, content_type, compression).await
    } else {
        chuncked_static_file_download(filename, content_type, compression).await
    }
}

// FIXME: support handling multiple compression methods
fn extract_compression_method(headers: &http::HeaderMap) -> Option<CompressionMethod> {
    if let Some(Ok(encodings)) = headers.get(header::ACCEPT_ENCODING).map(|v| v.to_str()) {
        for encoding in encodings.split(&[',', ' '][..]) {
            if let Ok(method) = encoding.parse() {
                return Some(method);
            }
        }
    }
    None
}

async fn handle_request(
    api: Arc<ApiConfig>,
    req: Request<Body>,
    peer: &std::net::SocketAddr,
) -> Result<Response<Body>, Error> {
    let (parts, body) = req.into_parts();
    let method = parts.method.clone();
    let (path, components) = normalize_uri_path(parts.uri.path())?;

    let comp_len = components.len();

    let query = parts.uri.query().unwrap_or_default();
    if path.len() + query.len() > MAX_URI_QUERY_LENGTH {
        return Ok(Response::builder()
            .status(StatusCode::URI_TOO_LONG)
            .body("".into())
            .unwrap());
    }

    let env_type = api.env_type();
    let mut rpcenv = RestEnvironment::new(env_type, Arc::clone(&api));

    rpcenv.set_client_ip(Some(*peer));

    let delay_unauth_time = std::time::Instant::now() + std::time::Duration::from_millis(3000);
    let access_forbidden_time = std::time::Instant::now() + std::time::Duration::from_millis(500);

    if comp_len >= 1 && components[0] == "api2" {
        if comp_len >= 2 {
            let format = components[1];

            let formatter: &dyn OutputFormatter = match format {
                "json" => JSON_FORMATTER,
                "extjs" => EXTJS_FORMATTER,
                _ => bail!("Unsupported output format '{}'.", format),
            };

            let mut uri_param = HashMap::new();
            let api_method = api.find_method(&components[2..], method.clone(), &mut uri_param);

            let mut auth_required = true;
            if let Some(api_method) = api_method {
                if let Permission::World = *api_method.access.permission {
                    auth_required = false; // no auth for endpoints with World permission
                }
            }

            let mut user_info: Box<dyn UserInformation + Send + Sync> = Box::new(EmptyUserInformation {});

            if auth_required {
                match api.check_auth(&parts.headers, &method).await {
                    Ok((authid, info)) => {
                        rpcenv.set_auth_id(Some(authid));
                        user_info = info;
                    }
                    Err(auth_err) => {
                        let err = match auth_err {
                            AuthError::Generic(err) => err,
                            AuthError::NoData => {
                                format_err!("no authentication credentials provided.")
                            }
                        };
                        // fixme: log Username??
                        rpcenv.log_failed_auth(None, &err.to_string());

                        // always delay unauthorized calls by 3 seconds (from start of request)
                        let err = http_err!(UNAUTHORIZED, "authentication failed - {}", err);
                        tokio::time::sleep_until(Instant::from_std(delay_unauth_time)).await;
                        return Ok(formatter.format_error(err));
                    }
                }
            }

            match api_method {
                None => {
                    let err = http_err!(NOT_FOUND, "Path '{}' not found.", path);
                    return Ok(formatter.format_error(err));
                }
                Some(api_method) => {
                    let auth_id = rpcenv.get_auth_id();
                    let user_info = user_info;

                    if !check_api_permission(
                        api_method.access.permission,
                        auth_id.as_deref(),
                        &uri_param,
                        user_info.as_ref(),
                    ) {
                        let err = http_err!(FORBIDDEN, "permission check failed");
                        tokio::time::sleep_until(Instant::from_std(access_forbidden_time)).await;
                        return Ok(formatter.format_error(err));
                    }

                    let result = if api_method.protected && env_type == RpcEnvironmentType::PUBLIC {
                        proxy_protected_request(api_method, parts, body, peer).await
                    } else {
                        handle_api_request(rpcenv, api_method, formatter, parts, body, uri_param)
                            .await
                    };

                    let mut response = match result {
                        Ok(resp) => resp,
                        Err(err) => formatter.format_error(err),
                    };

                    if let Some(auth_id) = auth_id {
                        response.extensions_mut().insert(AuthStringExtension(auth_id));
                    }

                    return Ok(response);
                }
            }
        }
    } else {
        // not Auth required for accessing files!

        if method != hyper::Method::GET {
            bail!("Unsupported HTTP method {}", method);
        }

        if comp_len == 0 {
            match api.check_auth(&parts.headers, &method).await {
                Ok((auth_id, _user_info)) => {
                    rpcenv.set_auth_id(Some(auth_id));
                    return Ok(api.get_index(rpcenv, parts).await);
                }
                Err(AuthError::Generic(_)) => {
                    tokio::time::sleep_until(Instant::from_std(delay_unauth_time)).await;
                }
                Err(AuthError::NoData) => {}
            }
            return Ok(api.get_index(rpcenv, parts).await);
        } else {
            let filename = api.find_alias(&components);
            let compression = extract_compression_method(&parts.headers);
            return handle_static_file_download(filename, compression).await;
        }
    }

    Err(http_err!(NOT_FOUND, "Path '{}' not found.", path))
}
