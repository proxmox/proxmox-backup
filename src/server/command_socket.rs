use anyhow::{bail, format_err, Error};

use futures::*;

use tokio::net::UnixListener;

use std::path::PathBuf;
use serde_json::Value;
use std::sync::Arc;
use std::os::unix::io::AsRawFd;
use nix::sys::socket;

/// Listens on a Unix Socket to handle simple command asynchronously
pub fn create_control_socket<P, F>(path: P, func: F) -> Result<impl Future<Output = ()>, Error>
where
    P: Into<PathBuf>,
    F: Fn(Value) -> Result<Value, Error> + Send + Sync + 'static,
{
    let path: PathBuf = path.into();

    let backup_user = crate::backup::backup_user()?;
    let backup_gid = backup_user.gid.as_raw();

    let mut socket = UnixListener::bind(&path)?;

    let func = Arc::new(func);

    let control_future = async move {
        loop {
            let (conn, _addr) = match socket.accept().await {
                Ok(data) => data,
                Err(err) => {
                    eprintln!("failed to accept on control socket {:?}: {}", path, err);
                    continue;
                }
            };

            let opt = socket::sockopt::PeerCredentials {};
            let cred = match socket::getsockopt(conn.as_raw_fd(), opt) {
                Ok(cred) => cred,
                Err(err) => {
                    eprintln!("no permissions - unable to read peer credential - {}", err);
                    continue;
                }
            };

            // check permissions (same gid, root user, or backup group)
            let mygid = unsafe { libc::getgid() };
            if !(cred.uid() == 0 || cred.gid() == mygid || cred.gid() == backup_gid) {
                eprintln!("no permissions for {:?}", cred);
                continue;
            }

            let (rx, mut tx) = tokio::io::split(conn);

            let abort_future = super::last_worker_future().map(|_| ());

            use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
            let func = Arc::clone(&func);
            let path = path.clone();
            tokio::spawn(futures::future::select(
                async move {
                    let mut rx = tokio::io::BufReader::new(rx);
                    let mut line = String::new();
                    loop {
                        line.clear();
                        match rx.read_line({ line.clear(); &mut line }).await {
                            Ok(0) => break,
                            Ok(_) => (),
                            Err(err) => {
                                eprintln!("control socket {:?} read error: {}", path, err);
                                return;
                            }
                        }

                        let response = match line.parse::<Value>() {
                            Ok(param) => match func(param) {
                                Ok(res) => format!("OK: {}\n", res),
                                Err(err) => format!("ERROR: {}\n", err),
                            }
                            Err(err) => format!("ERROR: {}\n", err),
                        };

                        if let Err(err) = tx.write_all(response.as_bytes()).await {
                            eprintln!("control socket {:?} write response error: {}", path, err);
                            return;
                        }
                    }
                }.boxed(),
                abort_future,
            ).map(|_| ()));
        }
    }.boxed();

    let abort_future = super::last_worker_future().map_err(|_| {});
    let task = futures::future::select(
        control_future,
        abort_future,
    ).map(|_: futures::future::Either<(Result<(), Error>, _), _>| ());

    Ok(task)
}


pub async fn send_command<P>(
    path: P,
    params: Value
) -> Result<Value, Error>
    where P: Into<PathBuf>,
{
    let path: PathBuf = path.into();

    tokio::net::UnixStream::connect(path)
        .map_err(move |err| format_err!("control socket connect failed - {}", err))
        .and_then(move |mut conn| {

            let mut command_string = params.to_string();
            command_string.push('\n');

            async move {
                use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

                conn.write_all(command_string.as_bytes()).await?;
                AsyncWriteExt::shutdown(&mut conn).await?;
                let mut rx = tokio::io::BufReader::new(conn);
                let mut data = String::new();
                if rx.read_line(&mut data).await? == 0 {
                    bail!("no response");
                }
                if data.starts_with("OK: ") {
                    match data[4..].parse::<Value>() {
                        Ok(v) => Ok(v),
                        Err(err) => bail!("unable to parse json response - {}", err),
                    }
                } else if data.starts_with("ERROR: ") {
                    bail!("{}", &data[7..]);
                } else {
                    bail!("unable to parse response: {}", data);
                }
            }
        }).await
}
