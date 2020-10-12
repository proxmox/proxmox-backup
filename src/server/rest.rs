use std::collections::HashMap;
use std::future::Future;
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::{bail, format_err, Error};
use futures::future::{self, FutureExt, TryFutureExt};
use futures::stream::TryStreamExt;
use hyper::header;
use hyper::http::request::Parts;
use hyper::{Body, Request, Response, StatusCode};
use serde_json::{json, Value};
use tokio::fs::File;
use tokio::time::Instant;
use url::form_urlencoded;

use proxmox::http_err;
use proxmox::api::{
    ApiHandler,
    ApiMethod,
    HttpError,
    Permission,
    RpcEnvironment,
    RpcEnvironmentType,
    check_api_permission,
};
use proxmox::api::schema::{
    ObjectSchema,
    parse_parameter_strings,
    parse_simple_value,
    verify_json_object,
};

use super::environment::RestEnvironment;
use super::formatter::*;
use super::ApiConfig;

use crate::auth_helpers::*;
use crate::api2::types::Userid;
use crate::tools;
use crate::tools::ticket::Ticket;
use crate::config::cached_user_info::CachedUserInfo;

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

        let config = Arc::clone(&self.api_config);
        let peer = self.peer;
        async move {
            match handle_request(config, req).await {
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
            }
        }
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
        .map_err(|err| http_err!(BAD_REQUEST, "Promlems reading request body: {}", err))
        .try_fold(Vec::new(), |mut acc, chunk| async move {
            if acc.len() + chunk.len() < 64*1024 { //fimxe: max request body size?
                acc.extend_from_slice(&*chunk);
                Ok(acc)
            } else {
                Err(http_err!(BAD_REQUEST, "Request body too large"))
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

fn get_index(
    userid: Option<Userid>,
    token: Option<String>,
    language: Option<String>,
    api: &Arc<ApiConfig>,
    parts: Parts,
) ->  Response<Body> {

    let nodename = proxmox::tools::nodename();
    let userid = userid.as_ref().map(|u| u.as_str()).unwrap_or("");

    let token = token.unwrap_or_else(|| String::from(""));

    let mut debug = false;
    let mut template_file = "index";

    if let Some(query_str) = parts.uri.query() {
        for (k, v) in form_urlencoded::parse(query_str.as_bytes()).into_owned() {
            if k == "debug" && v != "0" && v != "false" {
                debug = true;
            } else if k == "console" {
                template_file = "console";
            }
        }
    }

    let mut lang = String::from("");
    if let Some(language) = language {
        if Path::new(&format!("/usr/share/pbs-i18n/pbs-lang-{}.js", language)).exists() {
            lang = language;
        }
    }

    let data = json!({
        "NodeName": nodename,
        "UserName": userid,
        "CSRFPreventionToken": token,
        "language": lang,
        "debug": debug,
    });

    let (ct, index) = match api.render_template(template_file, &data) {
        Ok(index) => ("text/html", index),
        Err(err) => {
            ("text/plain", format!("Error rendering template: {}", err))
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, ct)
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
        .map_err(|err| http_err!(BAD_REQUEST, "File open failed: {}", err))?;

    let mut data: Vec<u8> = Vec::new();
    file.read_to_end(&mut data)
        .await
        .map_err(|err| http_err!(BAD_REQUEST, "File read failed: {}", err))?;

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
        .map_err(|err| http_err!(BAD_REQUEST, "File open failed: {}", err))?;

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

    let metadata = tokio::fs::metadata(filename.clone())
        .map_err(|err| http_err!(BAD_REQUEST, "File access problems: {}", err))
        .await?;

    if metadata.len() < 1024*32 {
        simple_static_file_download(filename).await
    } else {
        chuncked_static_file_download(filename).await
    }
}

fn extract_auth_data(headers: &http::HeaderMap) -> (Option<String>, Option<String>, Option<String>) {

    let mut ticket = None;
    let mut language = None;
    if let Some(raw_cookie) = headers.get("COOKIE") {
        if let Ok(cookie) = raw_cookie.to_str() {
            ticket = tools::extract_cookie(cookie, "PBSAuthCookie");
            language = tools::extract_cookie(cookie, "PBSLangCookie");
        }
    }

    let token = match headers.get("CSRFPreventionToken").map(|v| v.to_str()) {
        Some(Ok(v)) => Some(v.to_owned()),
        _ => None,
    };

    (ticket, token, language)
}

fn check_auth(
    method: &hyper::Method,
    ticket: &Option<String>,
    token: &Option<String>,
    user_info: &CachedUserInfo,
) -> Result<Userid, Error> {
    let ticket_lifetime = tools::ticket::TICKET_LIFETIME;

    let ticket = ticket.as_ref().map(String::as_str);
    let userid: Userid = Ticket::parse(&ticket.ok_or_else(|| format_err!("missing ticket"))?)?
        .verify_with_time_frame(public_auth_key(), "PBS", None, -300..ticket_lifetime)?;

    if !user_info.is_active_user(&userid) {
        bail!("user account disabled or expired.");
    }

    if method != hyper::Method::GET {
        if let Some(token) = token {
            verify_csrf_prevention_token(csrf_secret(), &userid, &token, -300, ticket_lifetime)?;
        } else {
            bail!("missing CSRF prevention token");
        }
    }

    Ok(userid)
}

async fn handle_request(api: Arc<ApiConfig>, req: Request<Body>) -> Result<Response<Body>, Error> {

    let (parts, body) = req.into_parts();

    let method = parts.method.clone();
    let (path, components) = tools::normalize_uri_path(parts.uri.path())?;

    let comp_len = components.len();

    //println!("REQUEST {} {}", method, path);
    //println!("COMPO {:?}", components);

    let env_type = api.env_type();
    let mut rpcenv = RestEnvironment::new(env_type);

    let user_info = CachedUserInfo::new()?;

    let delay_unauth_time = std::time::Instant::now() + std::time::Duration::from_millis(3000);
    let access_forbidden_time = std::time::Instant::now() + std::time::Duration::from_millis(500);

    if comp_len >= 1 && components[0] == "api2" {

        if comp_len >= 2 {

            let format = components[1];

            let formatter = match format {
                "json" => &JSON_FORMATTER,
                "extjs" => &EXTJS_FORMATTER,
                _ =>  bail!("Unsupported output format '{}'.", format),
            };

            let mut uri_param = HashMap::new();
            let api_method = api.find_method(&components[2..], method.clone(), &mut uri_param);

            let mut auth_required = true;
            if let Some(api_method) = api_method {
                if let Permission::World = *api_method.access.permission {
                    auth_required = false; // no auth for endpoints with World permission
                }
            }

            if auth_required {
                let (ticket, token, _) = extract_auth_data(&parts.headers);
                match check_auth(&method, &ticket, &token, &user_info) {
                    Ok(userid) => rpcenv.set_user(Some(userid.to_string())),
                    Err(err) => {
                        // always delay unauthorized calls by 3 seconds (from start of request)
                        let err = http_err!(UNAUTHORIZED, "authentication failed - {}", err);
                        tokio::time::delay_until(Instant::from_std(delay_unauth_time)).await;
                        return Ok((formatter.format_error)(err));
                    }
                }
            }

            match api_method {
                None => {
                    let err = http_err!(NOT_FOUND, "Path '{}' not found.", path);
                    return Ok((formatter.format_error)(err));
                }
                Some(api_method) => {
                    let user = rpcenv.get_user();
                    if !check_api_permission(api_method.access.permission, user.as_deref(), &uri_param, user_info.as_ref()) {
                        let err = http_err!(FORBIDDEN, "permission check failed");
                        tokio::time::delay_until(Instant::from_std(access_forbidden_time)).await;
                        return Ok((formatter.format_error)(err));
                    }

                    let result = if api_method.protected && env_type == RpcEnvironmentType::PUBLIC {
                        proxy_protected_request(api_method, parts, body).await
                    } else {
                        handle_api_request(rpcenv, api_method, formatter, parts, body, uri_param).await
                    };

                    if let Err(err) = result {
                        return Ok((formatter.format_error)(err));
                    }
                    return result;
                }
            }

        }
     } else {
        // not Auth required for accessing files!

        if method != hyper::Method::GET {
            bail!("Unsupported HTTP method {}", method);
        }

        if comp_len == 0 {
            let (ticket, token, language) = extract_auth_data(&parts.headers);
            if ticket != None {
                match check_auth(&method, &ticket, &token, &user_info) {
                    Ok(userid) => {
                        let new_token = assemble_csrf_prevention_token(csrf_secret(), &userid);
                        return Ok(get_index(Some(userid), Some(new_token), language, &api, parts));
                    }
                    _ => {
                        tokio::time::delay_until(Instant::from_std(delay_unauth_time)).await;
                        return Ok(get_index(None, None, language, &api, parts));
                    }
                }
            } else {
                return Ok(get_index(None, None, language, &api, parts));
            }
        } else {
            let filename = api.find_alias(&components);
            return handle_static_file_download(filename).await;
        }
    }

    Err(http_err!(NOT_FOUND, "Path '{}' not found.", path))
}
