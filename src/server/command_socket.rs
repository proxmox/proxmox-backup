use failure::*;

use futures::*;
use futures::stream::Stream;

use tokio::net::unix::UnixListener;
use tokio::io::AsyncRead;

use std::io::Write;

use std::path::PathBuf;
use serde_json::Value;
use std::sync::Arc;

/// Listens on a Unix Socket to handle simple command asynchronously
pub fn create_control_socket<P, F>(path: P, auto_remove: bool, f: F) -> Result<impl Future<Item=(), Error=()>, Error>
    where P: Into<PathBuf>,
          F: Send + Sync +'static + Fn(Value) -> Result<Value, Error>,
{
    let path: PathBuf = path.into();
    let path1: PathBuf = path.clone();

    if auto_remove { let _ = std::fs::remove_file(&path); }

    let socket = UnixListener::bind(&path)?;

    let f = Arc::new(f);
    let path2 = Arc::new(path);
    let path3 = path2.clone();

    let control_future = socket.incoming()
        .map_err(move |err| { eprintln!("failed to accept on control socket {:?}: {}", path2, err); })
        .for_each(move |conn| {
            let f1 = f.clone();

            let (rx, mut tx) = conn.split();
            let path = path3.clone();
            let path2 = path3.clone();

            tokio::io::lines(std::io::BufReader::new(rx))
                .map_err(move |err| { eprintln!("control socket {:?} read error: {}", path, err); })
                .and_then(move |cmd| {
                    let res = try_block!({
                        let param = match cmd.parse::<Value>() {
                            Ok(p) => p,
                            Err(err) => bail!("ERRER {}", err),
                        };

                        f1(param)
                    });

                    let resp = match res {
                        Ok(v) => format!("OK: {}\n", v),
                        Err(err) => format!("ERROR: {}\n", err),
                    };
                    Ok(resp)
                })
                .for_each(move |resp| {
                    tx.write_all(resp.as_bytes())
                        .map_err(|err| { eprintln!("control socket {:?} write response error: {}", path2, err); })
                })

        });

    let abort_future = super::last_worker_future().map_err(|_| {});
    // let task = control_future.select(abort_future).map(|_| {}).map_err(|_| {});
    let task = control_future.select(abort_future)
        .then(move |_| {
            if auto_remove { let _ = std::fs::remove_file(path1); }
            Ok(())
        });

    Ok(task)
}
