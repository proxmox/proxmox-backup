use std::path::PathBuf;
use anyhow::Error;
use futures::*;
use hyper::{Body, Response, StatusCode, header};
use proxmox::http_err;

pub async fn create_download_response(path: PathBuf) -> Result<Response<Body>, Error> {
    let file = tokio::fs::File::open(path.clone())
        .map_err(move |err| {
            match err.kind() {
                std::io::ErrorKind::NotFound => http_err!(NOT_FOUND, format!("open file {:?} failed - not found", path.clone())),
                _ => http_err!(BAD_REQUEST, format!("open file {:?} failed: {}", path.clone(), err)),
            }
        })
        .await?;

    let payload = tokio_util::codec::FramedRead::new(file, tokio_util::codec::BytesCodec::new())
        .map_ok(|bytes| hyper::body::Bytes::from(bytes.freeze()));

    let body = Body::wrap_stream(payload);

    // fixme: set other headers ?
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(body)
        .unwrap())
}
