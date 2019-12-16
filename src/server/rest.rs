use std::collections::HashMap;
use std::future::Future;
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use failure::*;
use futures::future::{self, FutureExt, TryFutureExt};
use futures::stream::TryStreamExt;
use hyper::header;
use hyper::http::request::Parts;
use hyper::{Body, Request, Response, StatusCode};
use serde_json::{json, Value};
use tokio::fs::File;
use tokio::time::Instant;
use url::form_urlencoded;

use proxmox::api::http_err;
use proxmox::api::{ApiHandler, ApiMethod, HttpError};
use proxmox::api::{RpcEnvironment, RpcEnvironmentType};
use proxmox::api::schema::{ObjectSchema, parse_simple_value, verify_json_object, parse_parameter_strings};

use super::environment::RestEnvironment;
use super::formatter::*;
use super::ApiConfig;

use crate::auth_helpers::*;
use crate::tools;

extern "C"  { fn tzset(); }

pub struct RestServer {
    pub api_config: Arc<ApiConfig>,
}

impl RestServer {

    pub fn new(api_config: ApiConfig) -> Self {
        Self { api_config: Arc::new(api_config) }
    }
}

impl tower_service::Service<&tokio_openssl::SslStream<tokio::net::TcpStream>> for RestServer {
    type Response = ApiService;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<ApiService, Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, ctx: &tokio_openssl::SslStream<tokio::net::TcpStream>) -> Self::Future {
        match ctx.get_ref().peer_addr() {
            Err(err) => {
                future::err(format_err!("unable to get peer address - {}", err)).boxed()
            }
            Ok(peer) => {
                future::ok(ApiService { peer, api_config: self.api_config.clone() }).boxed()
            }
        }
    }
}

impl tower_service::Service<&tokio::net::TcpStream> for RestServer {
    type Response = ApiService;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<ApiService, Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, ctx: &tokio::net::TcpStream) -> Self::Future {
        match ctx.peer_addr() {
            Err(err) => {
                future::err(format_err!("unable to get peer address - {}", err)).boxed()
            }
            Ok(peer) => {
                future::ok(ApiService { peer, api_config: self.api_config.clone() }).boxed()
            }
        }
    }
}

pub struct ApiService {
    pub peer: std::net::SocketAddr,
    pub api_config: Arc<ApiConfig>,
}

fn log_response(
    peer: &std::net::SocketAddr,
    method: hyper::Method,
    path: &str,
    resp: &Response<Body>,
) {

    if resp.extensions().get::<NoLogExtension>().is_some() { return; };

    let status = resp.status();

    if !(status.is_success() || status.is_informational()) {
        let reason = status.canonical_reason().unwrap_or("unknown reason");

        let mut message = "request failed";
        if let Some(data) = resp.extensions().get::<ErrorMessageExtension>() {
            message = &data.0;
        }

        log::error!("{} {}: {} {}: [client {}] {}", method.as_str(), path, status.as_str(), reason, peer, message);
    }
}

impl tower_service::Service<Request<Body>> for ApiService {
    type Response = Response<Body>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response<Body>, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let path = req.uri().path().to_owned();
        let method = req.method().clone();

        let peer = self.peer;
        handle_request(self.api_config.clone(), req)
            .map(move |result| match result {
                Ok(res) => {
                    log_response(&peer, method, &path, &res);
                    Ok::<_, Self::Error>(res)
                }
                Err(err) => {
                    if let Some(apierr) = err.downcast_ref::<HttpError>() {
                        let mut resp = Response::new(Body::from(apierr.message.clone()));
                        *resp.status_mut() = apierr.code;
                        log_response(&peer, method, &path, &resp);
                        Ok(resp)
                    } else {
                        let mut resp = Response::new(Body::from(err.to_string()));
                        *resp.status_mut() = StatusCode::BAD_REQUEST;
                        log_response(&peer, method, &path, &resp);
                        Ok(resp)
                    }
                }
            })
            .boxed()
    }
}

fn parse_query_parameters<S: 'static + BuildHasher + Send>(
    param_schema: &ObjectSchema,
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
            if k == "_dc" { continue; } // skip extjs "disable cache" parameter
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
    param_schema: &ObjectSchema,
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

    let body = req_body
        .map_err(|err| http_err!(BAD_REQUEST, format!("Promlems reading request body: {}", err)))
        .try_fold(Vec::new(), |mut acc, chunk| async move {
            if acc.len() + chunk.len() < 64*1024 { //fimxe: max request body size?
                acc.extend_from_slice(&*chunk);
                Ok(acc)
            } else {
                Err(http_err!(BAD_REQUEST, "Request body too large".to_string()))
            }
        }).await?;

    let utf8_data = std::str::from_utf8(&body)
        .map_err(|err| format_err!("Request body not uft8: {}", err))?;

    if is_json {
        let mut params: Value = serde_json::from_str(utf8_data)?;
        for (k, v) in uri_param {
            if let Some((_optional, prop_schema)) = param_schema.lookup(&k) {
                params[&k] = parse_simple_value(&v, prop_schema)?;
            }
        }
        verify_json_object(&params, param_schema)?;
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
) -> Result<Response<Body>, Error> {

    let mut uri_parts = parts.uri.clone().into_parts();

    uri_parts.scheme = Some(http::uri::Scheme::HTTP);
    uri_parts.authority = Some(http::uri::Authority::from_static("127.0.0.1:82"));
    let new_uri = http::Uri::from_parts(uri_parts).unwrap();

    parts.uri = new_uri;

    let request = Request::from_parts(parts, req_body);

    let reload_timezone = info.reload_timezone;

    let resp = hyper::client::Client::new()
        .request(request)
        .map_err(Error::from)
        .map_ok(|mut resp| {
            resp.extensions_mut().insert(NoLogExtension());
            resp
        })
        .await?;

    if reload_timezone { unsafe { tzset(); } }

    Ok(resp)
}

pub async fn handle_api_request<Env: RpcEnvironment, S: 'static + BuildHasher + Send>(
    mut rpcenv: Env,
    info: &'static ApiMethod,
    formatter: &'static OutputFormatter,
    parts: Parts,
    req_body: Body,
    uri_param: HashMap<String, String, S>,
) -> Result<Response<Body>, Error> {

    let delay_unauth_time = std::time::Instant::now() + std::time::Duration::from_millis(3000);

    let result = match info.handler {
        ApiHandler::AsyncHttp(handler) => {
            let params = parse_query_parameters(info.parameters, "", &parts, &uri_param)?;
            (handler)(parts, req_body, params, info, Box::new(rpcenv)).await
        }
        ApiHandler::Sync(handler) => {
            let params = get_request_parameters(info.parameters, parts, req_body, uri_param).await?;
            (handler)(params, info, &mut rpcenv)
                .map(|data| (formatter.format_data)(data, &rpcenv))
        }
        ApiHandler::Async(handler) => {
            let params = get_request_parameters(info.parameters, parts, req_body, uri_param).await?;
            (handler)(params, info, &mut rpcenv)
                .await
                .map(|data| (formatter.format_data)(data, &rpcenv))
        }
    };

    let resp = match result {
        Ok(resp) => resp,
        Err(err) => {
            if let Some(httperr) = err.downcast_ref::<HttpError>() {
                if httperr.code == StatusCode::UNAUTHORIZED {
                    tokio::time::delay_until(Instant::from_std(delay_unauth_time)).await;
                }
            }
            (formatter.format_error)(err)
        }
    };

    if info.reload_timezone { unsafe { tzset(); } }

    Ok(resp)
}

fn get_index(username: Option<String>, token: Option<String>) ->  Response<Body> {

    let nodename = proxmox::tools::nodename();
    let username = username.unwrap_or_else(|| String::from(""));

    let token = token.unwrap_or_else(|| String::from(""));

    let setup = json!({
        "Setup": { "auth_cookie_name": "PBSAuthCookie" },
        "NodeName": nodename,
        "UserName": username,
        "CSRFPreventionToken": token,
    });

    let index = format!(r###"
<!DOCTYPE html>
<html>
  <head>
    <meta http-equiv="Content-Type" content="text/html; charset=utf-8" />
    <meta http-equiv="X-UA-Compatible" content="IE=edge">
    <meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no">
    <title>Proxmox Backup Server</title>
    <link rel="icon" sizes="128x128" href="/images/logo-128.png" />
    <link rel="apple-touch-icon" sizes="128x128" href="/pve2/images/logo-128.png" />
    <link rel="stylesheet" type="text/css" href="/extjs/theme-crisp/resources/theme-crisp-all.css" />
    <link rel="stylesheet" type="text/css" href="/extjs/crisp/resources/charts-all.css" />
    <link rel="stylesheet" type="text/css" href="/fontawesome/css/font-awesome.css" />
    <script type='text/javascript'> function gettext(buf) {{ return buf; }} </script>
    <script type="text/javascript" src="/extjs/ext-all-debug.js"></script>
    <script type="text/javascript" src="/extjs/charts-debug.js"></script>
    <script type="text/javascript">
      Proxmox = {};
    </script>
    <script type="text/javascript" src="/widgettoolkit/proxmoxlib.js"></script>
    <script type="text/javascript" src="/extjs/locale/locale-en.js"></script>
    <script type="text/javascript">
      Ext.History.fieldid = 'x-history-field';
    </script>
    <script type="text/javascript" src="/js/proxmox-backup-gui.js"></script>
  </head>
  <body>
    <!-- Fields required for history management -->
    <form id="history-form" class="x-hidden">
      <input type="hidden" id="x-history-field"/>
    </form>
  </body>
</html>
"###, setup.to_string());

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html")
        .body(index.into())
        .unwrap()
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

async fn simple_static_file_download(filename: PathBuf) -> Result<Response<Body>, Error> {

    let (content_type, _nocomp) = extension_to_content_type(&filename);

    use tokio::io::AsyncReadExt;

    let mut file = File::open(filename)
        .await
        .map_err(|err| http_err!(BAD_REQUEST, format!("File open failed: {}", err)))?;

    let mut data: Vec<u8> = Vec::new();
    file.read_to_end(&mut data)
        .await
        .map_err(|err| http_err!(BAD_REQUEST, format!("File read failed: {}", err)))?;

    let mut response = Response::new(data.into());
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(content_type));
    Ok(response)
}

async fn chuncked_static_file_download(filename: PathBuf) -> Result<Response<Body>, Error> {
    let (content_type, _nocomp) = extension_to_content_type(&filename);

    let file = File::open(filename)
        .await
        .map_err(|err| http_err!(BAD_REQUEST, format!("File open failed: {}", err)))?;

    let payload = tokio_util::codec::FramedRead::new(file, tokio_util::codec::BytesCodec::new())
        .map_ok(|bytes| hyper::body::Bytes::from(bytes.freeze()));
    let body = Body::wrap_stream(payload);

    // fixme: set other headers ?
    Ok(Response::builder()
       .status(StatusCode::OK)
       .header(header::CONTENT_TYPE, content_type)
       .body(body)
       .unwrap()
    )
}

async fn handle_static_file_download(filename: PathBuf) ->  Result<Response<Body>, Error> {

    tokio::fs::metadata(filename.clone())
        .map_err(|err| http_err!(BAD_REQUEST, format!("File access problems: {}", err)))
        .and_then(|metadata| async move {
            if metadata.len() < 1024*32 {
                simple_static_file_download(filename).await
            } else {
                chuncked_static_file_download(filename).await
            }
        })
        .await
}

fn extract_auth_data(headers: &http::HeaderMap) -> (Option<String>, Option<String>) {

    let mut ticket = None;
    if let Some(raw_cookie) = headers.get("COOKIE") {
        if let Ok(cookie) = raw_cookie.to_str() {
            ticket = tools::extract_auth_cookie(cookie, "PBSAuthCookie");
        }
    }

    let token = match headers.get("CSRFPreventionToken").map(|v| v.to_str()) {
        Some(Ok(v)) => Some(v.to_owned()),
        _ => None,
    };

    (ticket, token)
}

fn check_auth(method: &hyper::Method, ticket: &Option<String>, token: &Option<String>) -> Result<String, Error> {

    let ticket_lifetime = tools::ticket::TICKET_LIFETIME;

    let username = match ticket {
        Some(ticket) => match tools::ticket::verify_rsa_ticket(public_auth_key(), "PBS", &ticket, None, -300, ticket_lifetime) {
            Ok((_age, Some(username))) => username.to_owned(),
            Ok((_, None)) => bail!("ticket without username."),
            Err(err) => return Err(err),
        }
        None => bail!("missing ticket"),
    };

    if method != hyper::Method::GET {
        if let Some(token) = token {
            println!("CSRF prevention token: {:?}", token);
            verify_csrf_prevention_token(csrf_secret(), &username, &token, -300, ticket_lifetime)?;
        } else {
            bail!("missing CSRF prevention token");
        }
    }

    Ok(username)
}

pub async fn handle_request(api: Arc<ApiConfig>, req: Request<Body>) -> Result<Response<Body>, Error> {

    let (parts, body) = req.into_parts();

    let method = parts.method.clone();
    let (path, components) = tools::normalize_uri_path(parts.uri.path())?;

    let comp_len = components.len();

    println!("REQUEST {} {}", method, path);
    println!("COMPO {:?}", components);

    let env_type = api.env_type();
    let mut rpcenv = RestEnvironment::new(env_type);

    let delay_unauth_time = std::time::Instant::now() + std::time::Duration::from_millis(3000);

    if comp_len >= 1 && components[0] == "api2" {

        if comp_len >= 2 {

            let format = components[1];

            let formatter = match format {
                "json" => &JSON_FORMATTER,
                "extjs" => &EXTJS_FORMATTER,
                _ =>  bail!("Unsupported output format '{}'.", format),
            };

            let mut uri_param = HashMap::new();

            if comp_len == 4 && components[2] == "access" && components[3] == "ticket" {
                // explicitly allow those calls without auth
            } else {
                let (ticket, token) = extract_auth_data(&parts.headers);
                match check_auth(&method, &ticket, &token) {
                    Ok(username) => {

                        // fixme: check permissions

                        rpcenv.set_user(Some(username));
                    }
                    Err(err) => {
                        // always delay unauthorized calls by 3 seconds (from start of request)
                        let err = http_err!(UNAUTHORIZED, format!("permission check failed - {}", err));
                        tokio::time::delay_until(Instant::from_std(delay_unauth_time)).await;
                        return Ok((formatter.format_error)(err));
                    }
                }
            }

            match api.find_method(&components[2..], method, &mut uri_param) {
                None => {
                    let err = http_err!(NOT_FOUND, "Path not found.".to_string());
                    return Ok((formatter.format_error)(err));
                }
                Some(api_method) => {
                    if api_method.protected && env_type == RpcEnvironmentType::PUBLIC {
                        return proxy_protected_request(api_method, parts, body).await;
                    } else {
                        return handle_api_request(rpcenv, api_method, formatter, parts, body, uri_param).await;
                    }
                }
            }
        }
     } else {
        // not Auth required for accessing files!

        if method != hyper::Method::GET {
            bail!("Unsupported HTTP method {}", method);
        }

        if comp_len == 0 {
            let (ticket, token) = extract_auth_data(&parts.headers);
            if ticket != None {
                match check_auth(&method, &ticket, &token) {
                    Ok(username) => {
                        let new_token = assemble_csrf_prevention_token(csrf_secret(), &username);
                        return Ok(get_index(Some(username), Some(new_token)));
                    }
                    _ => {
                        tokio::time::delay_until(Instant::from_std(delay_unauth_time)).await;
                        return Ok(get_index(None, None));
                    }
                }
            } else {
                return Ok(get_index(None, None));
            }
        } else {
            let filename = api.find_alias(&components);
            return handle_static_file_download(filename).await;
        }
    }

    Err(http_err!(NOT_FOUND, "Path not found.".to_string()))
}
