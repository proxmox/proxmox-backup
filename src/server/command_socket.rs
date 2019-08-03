use failure::*;

use futures::*;
use futures::stream::Stream;

use tokio::net::unix::UnixListener;
use tokio::io::AsyncRead;

use std::io::Write;

use std::path::PathBuf;
use serde_json::Value;
use std::sync::Arc;
use std::os::unix::io::AsRawFd;
use nix::sys::socket;

use proxmox::tools::try_block;

/// Listens on a Unix Socket to handle simple command asynchronously
pub fn create_control_socket<P, F>(path: P, f: F) -> Result<impl Future<Item=(), Error=()>, Error>
    where P: Into<PathBuf>,
          F: Send + Sync +'static + Fn(Value) -> Result<Value, Error>,
{
    let path: PathBuf = path.into();

    let socket = UnixListener::bind(&path)?;

    let f = Arc::new(f);
    let path2 = Arc::new(path);
    let path3 = path2.clone();

    let control_future = socket.incoming()
        .map_err(Error::from)
        .and_then(|conn| {
            // check permissions (same gid, or root user)
            let opt = socket::sockopt::PeerCredentials {};
            match socket::getsockopt(conn.as_raw_fd(), opt) {
                Ok(cred) => {
                    let mygid = unsafe { libc::getgid() };
                    if !(cred.uid() == 0 || cred.gid() == mygid) {
                        bail!("no permissions for {:?}", cred);
                    }
                }
                Err(err) => bail!("no permissions - unable to read peer credential - {}", err),
            }
            Ok(conn)
        })
        .map_err(move |err| { eprintln!("failed to accept on control socket {:?}: {}", path2, err); })
        .for_each(move |conn| {
            let f1 = f.clone();

            let (rx, mut tx) = conn.split();
            let path = path3.clone();
            let path2 = path3.clone();

            let abort_future = super::last_worker_future().map_err(|_| {});

            tokio::spawn(
                tokio::io::lines(std::io::BufReader::new(rx))
                    .map_err(move |err| { eprintln!("control socket {:?} read error: {}", path, err); })
                    .and_then(move |cmd| {
                        let res = try_block!({
                            let param = match cmd.parse::<Value>() {
                                Ok(p) => p,
                                Err(err) => bail!("unable to parse json value - {}", err),
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
                    .select(abort_future)
                    .then(move |_| { Ok(()) })
            )
        });

    let abort_future = super::last_worker_future().map_err(|_| {});
    let task = control_future.select(abort_future)
        .then(move |_| { Ok(()) });

    Ok(task)
}


pub fn send_command<P>(
    path: P,
    params: Value
) -> impl Future<Item=Value, Error=Error>
    where P: Into<PathBuf>,

{
    let path: PathBuf = path.into();

    tokio::net::UnixStream::connect(path)
        .map_err(move |err| format_err!("control socket connect failed - {}", err))
        .and_then(move |conn| {

            let (rx, tx) = conn.split();

            let mut command_string = params.to_string();
            command_string.push('\n');

            tokio::io::write_all(tx, command_string)
                .and_then(|(tx,_)| tokio::io::shutdown(tx))
                .map_err(|err| format_err!("control socket write error - {}", err))
                .and_then(move |_| {
                    tokio::io::lines(std::io::BufReader::new(rx))
                        .into_future()
                        .then(|test| {
                            match test {
                                Ok((Some(data), _)) => {
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
                                Ok((None, _)) => {
                                    bail!("no response");
                                }
                                Err((err, _)) => Err(Error::from(err)),
                            }
                        })
                })
        })
}
