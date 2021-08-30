use std::io::{Read, Seek};
use std::path::PathBuf;

use anyhow::Error;
use futures::stream::TryStreamExt;
use hyper::{Body, Response, StatusCode, header};

use proxmox::http_bail;

use pbs_datastore::catalog::{CatalogReader, DirEntryAttribute};

use crate::api2::types::ArchiveEntry;

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

/// Returns the list of content of the given path
pub fn list_dir_content<R: Read + Seek>(
    reader: &mut CatalogReader<R>,
    path: &[u8],
) -> Result<Vec<ArchiveEntry>, Error> {
    let dir = reader.lookup_recursive(path)?;
    let mut res = vec![];
    let mut path = path.to_vec();
    if !path.is_empty() && path[0] == b'/' {
        path.remove(0);
    }

    for direntry in reader.read_dir(&dir)? {
        let mut components = path.clone();
        components.push(b'/');
        components.extend(&direntry.name);
        let mut entry = ArchiveEntry::new(&components, Some(&direntry.attr));
        if let DirEntryAttribute::File { size, mtime } = direntry.attr {
            entry.size = size.into();
            entry.mtime = mtime.into();
        }
        res.push(entry);
    }

    Ok(res)
}
