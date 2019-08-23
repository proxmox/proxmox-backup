use failure::*;

use futures::*;

use tokio::net::unix::UnixListener;

use std::path::PathBuf;
use serde_json::Value;
use std::sync::Arc;
use std::os::unix::io::AsRawFd;
use nix::sys::socket;

/// Listens on a Unix Socket to handle simple command asynchronously
pub fn create_control_socket<P, F>(path: P, f: F) -> Result<impl Future<Output = ()>, Error>
where
    P: Into<PathBuf>,
    F: Fn(Value) -> Result<Value, Error> + Send + Sync + 'static,
{
    let path: PathBuf = path.into();

    let socket = UnixListener::bind(&path)?;

    let f = Arc::new(f);
    let path2 = Arc::new(path);
    let path3 = path2.clone();

    let control_future = socket.incoming()
        .map_err(Error::from)
        .and_then(|conn| {
            use futures::future::{err, ok};

            // check permissions (same gid, or root user)
            let opt = socket::sockopt::PeerCredentials {};
            match socket::getsockopt(conn.as_raw_fd(), opt) {
                Ok(cred) => {
                    let mygid = unsafe { libc::getgid() };
                    if !(cred.uid() == 0 || cred.gid() == mygid) {
                        return err(format_err!("no permissions for {:?}", cred));
                    }
                }
                Err(e) => {
                    return err(format_err!(
                        "no permissions - unable to read peer credential - {}",
                        e,
                    ));
                }
            }
            ok(conn)
        })
        .map_err(move |err| { eprintln!("failed to accept on control socket {:?}: {}", path2, err); })
        .try_for_each(move |conn| {
            let f = Arc::clone(&f);

            let (rx, mut tx) = conn.split();
            let path = path3.clone();

            let abort_future = super::last_worker_future().map(|_| ());

            use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
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
                            Ok(param) => match f(param) {
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
            futures::future::ok(())
        });

    let abort_future = super::last_worker_future().map_err(|_| {});
    let task = futures::future::select(
        control_future,
        abort_future,
    ).map(|_| ());

    Ok(task)
}


pub fn send_command<P>(
    path: P,
    params: Value
) -> impl Future<Output = Result<Value, Error>>
    where P: Into<PathBuf>,

{
    let path: PathBuf = path.into();

    tokio::net::UnixStream::connect(path)
        .map_err(move |err| format_err!("control socket connect failed - {}", err))
        .and_then(move |conn| {

            let (rx, mut tx) = conn.split();

            let mut command_string = params.to_string();
            command_string.push('\n');

            async move {
                use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

                tx.write_all(command_string.as_bytes()).await?;
                tx.shutdown().await?;
                let mut rx = tokio::io::BufReader::new(rx);
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
        })
}
