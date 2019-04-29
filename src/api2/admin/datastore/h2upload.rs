use failure::*;

use futures::{Future, Stream};
use h2::server;
use hyper::header::{HeaderValue, UPGRADE};
use hyper::{Body, Response, StatusCode};
use hyper::http::request::Parts;
use hyper::rt;

use serde_json::Value;

use crate::api_schema::router::*;
use crate::api_schema::*;

pub fn api_method_upgrade_h2upload() -> ApiAsyncMethod {
    ApiAsyncMethod::new(
        upgrade_h2upload,
        ObjectSchema::new("Experimental h2 server")
            .required("store", StringSchema::new("Datastore name.")),
    )
}

fn upgrade_h2upload(
    parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiAsyncMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<BoxFut, Error> {
    let expected_protocol: &'static str = "proxmox-backup-protocol-h2";

    let protocols = parts
        .headers
        .get("UPGRADE")
        .ok_or_else(|| format_err!("missing Upgrade header"))?
        .to_str()?;

    if protocols != expected_protocol {
        bail!("invalid protocol name");
    }

    rt::spawn(
        req_body
            .on_upgrade()
            .map_err(|e| Error::from(e))
            .and_then(move |conn| {
                println!("upgrade done");
                server::handshake(conn)
                    .and_then(|h2| {
                        println!("Accept h2");
                        // Accept all inbound HTTP/2.0 streams sent over the
                        // connection.
                        h2.for_each(|(request, mut respond)| {
                            println!("Received request: {:?}", request);

                            // Build a response with no body
                            let response = Response::builder()
                                .status(StatusCode::OK)
                                .body(())
                                .unwrap();

                            // Send the response back to the client
                            respond.send_response(response, true)
                                .unwrap();

                            Ok(())
                        })
                    })
                    .map_err(Error::from)
            })
            .map_err(|e| eprintln!("error during upgrade: {}", e))
    );

    Ok(Box::new(futures::future::ok(
        Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header(UPGRADE, HeaderValue::from_static(expected_protocol))
            .body(Body::empty())
            .unwrap()
    )))
}
