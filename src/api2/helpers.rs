use std::path::PathBuf;

use anyhow::Error;
use futures::stream::TryStreamExt;
use hyper::{header, Body, Response, StatusCode};

use proxmox_router::http_bail;

pub async fn create_download_response(path: PathBuf) -> Result<Response<Body>, Error> {
    let file = match tokio::fs::File::open(path.clone()).await {
        Ok(file) => file,
        Err(ref err) if err.kind() == std::io::ErrorKind::NotFound => {
            http_bail!(NOT_FOUND, "open file {:?} failed - not found", path);
        }
        Err(err) => http_bail!(BAD_REQUEST, "open file {:?} failed: {}", path, err),
    };

    let payload = tokio_util::codec::FramedRead::new(file, tokio_util::codec::BytesCodec::new())
        .map_ok(|bytes| bytes.freeze());

    let body = Body::wrap_stream(payload);

    // fixme: set other headers ?
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(body)
        .unwrap())
}
