use crate::tools;
use crate::api::schema::*;
use crate::api::router::*;
use crate::api::config::*;
use crate::auth_helpers::*;
use super::environment::RestEnvironment;
use super::formatter::*;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::collections::HashMap;

use failure::*;
use serde_json::{json, Value};
use url::form_urlencoded;

use futures::future::{self, Either};
//use tokio::prelude::*;
//use tokio::timer::Delay;
use tokio::fs::File;
//use bytes::{BytesMut, BufMut};

//use hyper::body::Payload;
use hyper::http::request::Parts;
use hyper::{Body, Request, Response, StatusCode};
use hyper::service::{Service, NewService};
use hyper::rt::{Future, Stream};
use hyper::header;

extern "C"  { fn tzset(); }

pub struct RestServer {
    pub api_config: Arc<ApiConfig>,
}

impl RestServer {

    pub fn new(api_config: ApiConfig) -> Self {
        Self { api_config: Arc::new(api_config) }
    }
}

impl NewService for RestServer
{
    type ReqBody = Body;
    type ResBody = Body;
    type Error = hyper::Error;
    type InitError = hyper::Error;
    type Service = ApiService;
    type Future = Box<Future<Item = Self::Service, Error = Self::InitError> + Send>;
    fn new_service(&self) -> Self::Future {
        Box::new(future::ok(ApiService { api_config: self.api_config.clone() }))
    }
}

pub struct ApiService {
    pub api_config: Arc<ApiConfig>,
}

impl ApiService {
    fn log_response(path: &str, resp: &Response<Body>) {
        let status = resp.status();
        if !status.is_success() {
            let reason = status.canonical_reason().unwrap_or("unknown reason");
            let client = "unknown"; // fixme: howto get peer_addr ?
            let message = "request failed";

            log::error!("{}: {} {}: [client {}] {}", path, status.as_str(), reason, client, message);
        }
    }
}

impl Service for ApiService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = hyper::Error;
    type Future = Box<Future<Item = Response<Body>, Error = Self::Error> + Send>;

    fn call(&mut self, req: Request<Self::ReqBody>) -> Self::Future {
        let path = req.uri().path().to_owned();
        Box::new(handle_request(self.api_config.clone(), req).then(move |result| {
            match result {
                Ok(res) => {
                    Self::log_response(&path, &res);
                    Ok::<_, hyper::Error>(res)
                }
                Err(err) => {
                    if let Some(apierr) = err.downcast_ref::<HttpError>() {
                        let mut resp = Response::new(Body::from(apierr.message.clone()));
                        *resp.status_mut() = apierr.code;
                        Self::log_response(&path, &resp);
                        Ok(resp)
                    } else {
                        let mut resp = Response::new(Body::from(err.to_string()));
                        *resp.status_mut() = StatusCode::BAD_REQUEST;
                        Self::log_response(&path, &resp);
                        Ok(resp)
                    }
                }
            }
        }))
    }
}

fn get_request_parameters_async(
    info: &'static ApiMethod,
    parts: Parts,
    req_body: Body,
    uri_param: HashMap<String, String>,
) -> Box<Future<Item = Value, Error = failure::Error> + Send>
{
    let resp = req_body
        .map_err(|err| http_err!(BAD_REQUEST, format!("Promlems reading request body: {}", err)))
        .fold(Vec::new(), |mut acc, chunk| {
            if acc.len() + chunk.len() < 64*1024 { //fimxe: max request body size?
                acc.extend_from_slice(&*chunk);
                Ok(acc)
            }
            else { Err(http_err!(BAD_REQUEST, format!("Request body too large"))) }
        })
        .and_then(move |body| {

            let utf8 = std::str::from_utf8(&body)?;

            let mut param_list: Vec<(String, String)> = vec![];

            if utf8.len() > 0 {
                for (k, v) in form_urlencoded::parse(utf8.as_bytes()).into_owned() {
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

            let params = parse_parameter_strings(&param_list, &info.parameters, true)?;

            Ok(params)
        });

    Box::new(resp)
}

fn proxy_protected_request(
    info: &'static ApiMethod,
    mut parts: Parts,
    req_body: Body,
) -> BoxFut
{

    let mut uri_parts = parts.uri.clone().into_parts();

    uri_parts.scheme = Some(http::uri::Scheme::HTTP);
    uri_parts.authority = Some(http::uri::Authority::from_static("127.0.0.1:82"));
    let new_uri = http::Uri::from_parts(uri_parts).unwrap();

    parts.uri = new_uri;

    let request = Request::from_parts(parts, req_body);

    let resp = hyper::client::Client::new()
        .request(request)
        .map_err(|e| Error::from(e));

    let resp = if info.reload_timezone {
        Either::A(resp.then(|resp| {unsafe { tzset() }; resp }))
    } else {
        Either::B(resp)
    };

    return Box::new(resp);
}

fn handle_sync_api_request(
    mut rpcenv: RestEnvironment,
    info: &'static ApiMethod,
    formatter: &'static OutputFormatter,
    parts: Parts,
    req_body: Body,
    uri_param: HashMap<String, String>,
) -> BoxFut
{
    let params = get_request_parameters_async(info, parts, req_body, uri_param);

    let delay_unauth_time = std::time::Instant::now() + std::time::Duration::from_millis(3000);

    let resp = params
        .and_then(move |params| {
            let mut delay = false;
            let resp = match (info.handler)(params, info, &mut rpcenv) {
                Ok(data) => (formatter.format_result)(data, &rpcenv),
                Err(err) => {
                    if let Some(httperr) = err.downcast_ref::<HttpError>() {
                        if httperr.code == StatusCode::UNAUTHORIZED { delay = true; }
                    }
                    (formatter.format_error)(err)
                }
            };

            if info.reload_timezone {
                unsafe { tzset() };
            }

            if delay {
                let delayed_response = tokio::timer::Delay::new(delay_unauth_time)
                    .map_err(|err| http_err!(INTERNAL_SERVER_ERROR, format!("tokio timer delay error: {}", err)))
                    .and_then(|_| Ok(resp));

                Either::A(delayed_response)
            } else {
                Either::B(future::ok(resp))
            }
        });

    Box::new(resp)
}

fn handle_async_api_request(
    mut rpcenv: RestEnvironment,
    info: &'static ApiAsyncMethod,
    formatter: &'static OutputFormatter,
    parts: Parts,
    req_body: Body,
    uri_param: HashMap<String, String>,
) -> BoxFut
{
    // fixme: convert parameters to Json
    let mut param_list: Vec<(String, String)> = vec![];

    if let Some(query_str) = parts.uri.query() {
        for (k, v) in form_urlencoded::parse(query_str.as_bytes()).into_owned() {
            if k == "_dc" { continue; } // skip extjs "disable cache" parameter
            param_list.push((k, v));
        }
    }

    for (k, v) in uri_param {
        param_list.push((k.clone(), v.clone()));
    }

    let params = match parse_parameter_strings(&param_list, &info.parameters, true) {
        Ok(v) => v,
        Err(err) => {
            let resp = (formatter.format_error)(Error::from(err));
            return Box::new(future::ok(resp));
        }
    };

    match (info.handler)(parts, req_body, params, info, &mut rpcenv) {
        Ok(future) => future,
        Err(err) => {
            let resp = (formatter.format_error)(Error::from(err));
            Box::new(future::ok(resp))
        }
    }
}

fn get_index() ->  BoxFut {

    let nodename = tools::nodename();
    let username = ""; // fixme: implement real auth
    let token = "";

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

    let resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html")
        .body(index.into())
        .unwrap();

    Box::new(future::ok(resp))
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

fn simple_static_file_download(filename: PathBuf) ->  BoxFut {

    let (content_type, _nocomp) = extension_to_content_type(&filename);

    Box::new(File::open(filename)
        .map_err(|err| http_err!(BAD_REQUEST, format!("File open failed: {}", err)))
        .and_then(move |file| {
            let buf: Vec<u8> = Vec::new();
            tokio::io::read_to_end(file, buf)
                .map_err(|err| http_err!(BAD_REQUEST, format!("File read failed: {}", err)))
                .and_then(move |data| {
                    let mut response = Response::new(data.1.into());
                    response.headers_mut().insert(
                        header::CONTENT_TYPE,
                        header::HeaderValue::from_static(content_type));
                    Ok(response)
                })
        }))
}

fn chuncked_static_file_download(filename: PathBuf) ->  BoxFut {

    let (content_type, _nocomp) = extension_to_content_type(&filename);

    Box::new(File::open(filename)
        .map_err(|err| http_err!(BAD_REQUEST, format!("File open failed: {}", err)))
        .and_then(move |file| {
            let payload = tokio::codec::FramedRead::new(file, tokio::codec::BytesCodec::new()).
                map(|bytes| {
                    //sigh - howto avoid copy here? or the whole map() ??
                    hyper::Chunk::from(bytes.to_vec())
                });
            let body = Body::wrap_stream(payload);

            // fixme: set other headers ?
            Ok(Response::builder()
               .status(StatusCode::OK)
               .header(header::CONTENT_TYPE, content_type)
               .body(body)
               .unwrap())
        }))
}

fn handle_static_file_download(filename: PathBuf) ->  BoxFut {

    let response = tokio::fs::metadata(filename.clone())
        .map_err(|err| http_err!(BAD_REQUEST, format!("File access problems: {}", err)))
        .and_then(|metadata| {
            if metadata.len() < 1024*32 {
                Either::A(simple_static_file_download(filename))
            } else {
                Either::B(chuncked_static_file_download(filename))
             }
        });

    return Box::new(response);
}

pub fn handle_request(api: Arc<ApiConfig>, req: Request<Body>) -> BoxFut {

    let (parts, body) = req.into_parts();

    let method = parts.method.clone();
    let path = parts.uri.path();

    // normalize path
    // do not allow ".", "..", or hidden files ".XXXX"
    // also remove empty path components

    let items = path.split('/');
    let mut path = String::new();
    let mut components = vec![];

    for name in items {
        if name.is_empty() { continue; }
        if name.starts_with(".") {
            return Box::new(future::err(http_err!(BAD_REQUEST, "Path contains illegal components.".to_string())));
        }
        path.push('/');
        path.push_str(name);
        components.push(name);
    }

    let comp_len = components.len();

    println!("REQUEST {} {}", method, path);
    println!("COMPO {:?}", components);

    let env_type = api.env_type();
    let mut rpcenv = RestEnvironment::new(env_type);

    let delay_unauth_time = std::time::Instant::now() + std::time::Duration::from_millis(3000);

    if let Some(raw_cookie) = parts.headers.get("COOKIE") {
        if let Ok(cookie) = raw_cookie.to_str() {
            if let Some(ticket) = tools::extract_auth_cookie(cookie, "PBSAuthCookie") {
                if let Ok((_, Some(username))) = tools::ticket::verify_rsa_ticket(
                    public_auth_key(), "PBS", &ticket, None, -300, 3600*2) {
                    rpcenv.set_user(Some(username));
                }
            }
        }
    }


    if comp_len >= 1 && components[0] == "api2" {
        println!("GOT API REQUEST");
        if comp_len >= 2 {
            let format = components[1];
            let formatter = match format {
                "json" => &JSON_FORMATTER,
                "extjs" => &EXTJS_FORMATTER,
                _ =>  {
                    return Box::new(future::err(http_err!(BAD_REQUEST, format!("Unsupported output format '{}'.", format))));
                }
            };

            let mut uri_param = HashMap::new();

            if comp_len == 4 && components[2] == "access" && components[3] == "ticket" {
                // explicitly allow those calls without auth
            } else {
                if let Some(_username) = rpcenv.get_user() {
                    // fixme: check permissions
                } else {
                    // always delay unauthorized calls by 3 seconds (from start of request)
                    let resp = (formatter.format_error)(http_err!(UNAUTHORIZED, "permission check failed.".into()));
                    let delayed_response = tokio::timer::Delay::new(delay_unauth_time)
                        .map_err(|err| http_err!(INTERNAL_SERVER_ERROR, format!("tokio timer delay error: {}", err)))
                        .and_then(|_| Ok(resp));

                    return Box::new(delayed_response);
                }
            }

            match api.find_method(&components[2..], method, &mut uri_param) {
                MethodDefinition::None => {}
                MethodDefinition::Simple(api_method) => {
                    if api_method.protected && env_type == RpcEnvironmentType::PUBLIC {
                        return proxy_protected_request(api_method, parts, body);
                    } else {
                        return handle_sync_api_request(rpcenv, api_method, formatter, parts, body, uri_param);
                    }
                }
                MethodDefinition::Async(async_method) => {
                    return handle_async_api_request(rpcenv, async_method, formatter, parts, body, uri_param);
                }
            }
        }
    } else {
        // not Auth for accessing files!

        if comp_len == 0 {
            return get_index();
        } else {
            let filename = api.find_alias(&components);
            return handle_static_file_download(filename);
        }
    }

    Box::new(future::err(http_err!(NOT_FOUND, "Path not found.".to_string())))
}
