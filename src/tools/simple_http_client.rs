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

use crate::tools::PROXMOX_BACKUP_TCP_KEEPALIVE_TIME;

/// Asyncrounous HTTP client implementation
pub struct SimpleHttp {
    client: Client<HttpsConnector, Body>,
    proxy_authorization: Option<String>, // Proxy-Authorization header value
    user_agent: Option<String>,
}

impl SimpleHttp {

    pub const DEFAULT_USER_AGENT_STRING: &'static str = "proxmox-backup-client/1.0";

    pub fn new(proxy_config: Option<ProxyConfig>) -> Self {
        let ssl_connector = SslConnector::builder(SslMethod::tls()).unwrap().build();
        Self::with_ssl_connector(ssl_connector, proxy_config)
    }

    pub fn with_ssl_connector(ssl_connector: SslConnector, proxy_config: Option<ProxyConfig>) -> Self {

        let mut proxy_authorization = None;
        if let Some(ref proxy_config) = proxy_config {
            if !proxy_config.force_connect {
               proxy_authorization = proxy_config.authorization.clone();
            }
        }

        let connector = HttpConnector::new();
        let mut https = HttpsConnector::with_connector(connector, ssl_connector, PROXMOX_BACKUP_TCP_KEEPALIVE_TIME);
        if let Some(proxy_config) = proxy_config {
            https.set_proxy(proxy_config);
        }
        let client = Client::builder().build(https);
        Self { client, proxy_authorization, user_agent: None }
    }

    pub fn set_user_agent(&mut self, user_agent: &str) -> Result<(), Error> {
        self.user_agent = Some(user_agent.to_owned());
        Ok(())
    }

    fn add_proxy_headers(&self, request: &mut Request<Body>) -> Result<(), Error> {
        if request.uri().scheme() != Some(&http::uri::Scheme::HTTPS) {
            if let Some(ref authorization) = self.proxy_authorization {
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
        let user_agent = if let Some(ref user_agent) = self.user_agent {
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
