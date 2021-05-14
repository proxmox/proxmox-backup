use anyhow::{Error, format_err, bail};
use std::collections::HashMap;

use hyper::Body;
use hyper::client::{Client, HttpConnector};
use http::{Request, Response, HeaderValue};
use openssl::ssl::{SslConnector, SslMethod};
use futures::*;

use proxmox_http::http::{
    ProxyConfig,
    client::HttpsConnector,
};

/// Options for a SimpleHttp client.
#[derive(Default)]
pub struct SimpleHttpOptions {
    /// Proxy configuration
    pub proxy_config: Option<ProxyConfig>,
    /// `User-Agent` header value, defaults to `proxmox-simple-http-client/0.1`
    pub user_agent: Option<String>,
    /// TCP keepalive time, defaults to 7200
    pub tcp_keepalive: Option<u32>,
}

impl SimpleHttpOptions {
    fn get_proxy_authorization(&self) -> Option<String> {
        if let Some(ref proxy_config) = self.proxy_config {
            if !proxy_config.force_connect {
               return proxy_config.authorization.clone();
            }
        }

        None
    }
}

/// Asyncrounous HTTP client implementation
pub struct SimpleHttp {
    client: Client<HttpsConnector, Body>,
    options: SimpleHttpOptions,
}

impl SimpleHttp {
    pub const DEFAULT_USER_AGENT_STRING: &'static str = "proxmox-simple-http-client/0.1";

    pub fn new() -> Self {
        Self::with_options(SimpleHttpOptions::default())
    }

    pub fn with_options(options: SimpleHttpOptions) -> Self {
        let ssl_connector = SslConnector::builder(SslMethod::tls()).unwrap().build();
        Self::with_ssl_connector(ssl_connector, options)
    }

    pub fn with_ssl_connector(ssl_connector: SslConnector, options: SimpleHttpOptions) -> Self {
        let connector = HttpConnector::new();
        let mut https = HttpsConnector::with_connector(connector, ssl_connector, options.tcp_keepalive.unwrap_or(7200));
        if let Some(ref proxy_config) = options.proxy_config {
            https.set_proxy(proxy_config.clone());
        }
        let client = Client::builder().build(https);
        Self { client, options }
    }

    pub fn set_user_agent(&mut self, user_agent: &str) -> Result<(), Error> {
        self.options.user_agent = Some(user_agent.to_owned());
        Ok(())
    }

    fn add_proxy_headers(&self, request: &mut Request<Body>) -> Result<(), Error> {
        if request.uri().scheme() != Some(&http::uri::Scheme::HTTPS) {
            if let Some(ref authorization) = self.options.get_proxy_authorization() {
                request
                    .headers_mut()
                    .insert(
                        http::header::PROXY_AUTHORIZATION,
                        HeaderValue::from_str(authorization)?,
                    );
            }
        }
        Ok(())
    }

    pub async fn request(&self, mut request: Request<Body>) -> Result<Response<Body>, Error> {
        let user_agent = if let Some(ref user_agent) = self.options.user_agent {
            HeaderValue::from_str(&user_agent)?
        } else {
            HeaderValue::from_str(Self::DEFAULT_USER_AGENT_STRING)?
        };

        request.headers_mut().insert(hyper::header::USER_AGENT, user_agent);

        self.add_proxy_headers(&mut request)?;

        self.client.request(request)
            .map_err(Error::from)
            .await
    }

    pub async fn post(
        &mut self,
        uri: &str,
        body: Option<String>,
        content_type: Option<&str>,
    ) -> Result<Response<Body>, Error> {

        let body = if let Some(body) = body {
            Body::from(body)
        } else {
            Body::empty()
        };
        let content_type = content_type.unwrap_or("application/json");

        let request = Request::builder()
            .method("POST")
            .uri(uri)
            .header(hyper::header::CONTENT_TYPE, content_type)
            .body(body)?;

        self.request(request).await
    }

    pub async fn get_string(
        &mut self,
        uri: &str,
        extra_headers: Option<&HashMap<String, String>>,
    ) -> Result<String, Error> {

        let mut request = Request::builder()
            .method("GET")
            .uri(uri);

        if let Some(hs) = extra_headers {
            for (h, v) in hs.iter() {
                request = request.header(h, v);
            }
        }

        let request = request.body(Body::empty())?;

        let res = self.request(request).await?;

        let status = res.status();
        if !status.is_success() {
            bail!("Got bad status '{}' from server", status)
        }

        Self::response_body_string(res).await
    }

    pub async fn response_body_string(res: Response<Body>) -> Result<String, Error> {
        let buf = hyper::body::to_bytes(res).await?;
        String::from_utf8(buf.to_vec())
            .map_err(|err| format_err!("Error converting HTTP result data: {}", err))
    }
}
