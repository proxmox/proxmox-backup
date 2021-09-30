use anyhow::{bail, format_err, Error};

use std::collections::HashMap;
use std::os::unix::io::AsRawFd;
use std::path::{PathBuf, Path};
use std::sync::Arc;

use futures::*;
use tokio::net::UnixListener;
use serde::Serialize;
use serde_json::Value;
use nix::sys::socket;
use nix::unistd::Gid;

// Listens on a Unix Socket to handle simple command asynchronously
fn create_control_socket<P, F>(path: P, gid: Gid, func: F) -> Result<impl Future<Output = ()>, Error>
where
    P: Into<PathBuf>,
    F: Fn(Value) -> Result<Value, Error> + Send + Sync + 'static,
{
    let path: PathBuf = path.into();

    let gid = gid.as_raw();

    let socket = UnixListener::bind(&path)?;

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
            if !(cred.uid() == 0 || cred.gid() == mygid || cred.gid() == gid) {
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

    let abort_future = crate::last_worker_future().map_err(|_| {});
    let task = futures::future::select(
        control_future,
        abort_future,
    ).map(|_: futures::future::Either<(Result<(), Error>, _), _>| ());

    Ok(task)
}

/// Send a command to the specified socket
pub async fn send_command<P, T>(path: P, params: &T) -> Result<Value, Error>
where
    P: AsRef<Path>,
    T: ?Sized + Serialize,
{
    let mut command_string = serde_json::to_string(params)?;
    command_string.push('\n');
    send_raw_command(path.as_ref(), &command_string).await
}

/// Send a raw command (string) to the specified socket
pub async fn send_raw_command<P>(path: P, command_string: &str) -> Result<Value, Error>
where
    P: AsRef<Path>,
{
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    let mut conn = tokio::net::UnixStream::connect(path)
        .map_err(move |err| format_err!("control socket connect failed - {}", err))
        .await?;

    conn.write_all(command_string.as_bytes()).await?;
    if !command_string.as_bytes().ends_with(b"\n") {
        conn.write_all(b"\n").await?;
    }

    AsyncWriteExt::shutdown(&mut conn).await?;
    let mut rx = tokio::io::BufReader::new(conn);
    let mut data = String::new();
    if rx.read_line(&mut data).await? == 0 {
        bail!("no response");
    }
    if let Some(res) = data.strip_prefix("OK: ") {
        match res.parse::<Value>() {
            Ok(v) => Ok(v),
            Err(err) => bail!("unable to parse json response - {}", err),
        }
    } else if let Some(err) = data.strip_prefix("ERROR: ") {
        bail!("{}", err);
    } else {
        bail!("unable to parse response: {}", data);
    }
}

// A callback for a specific commando socket.
type CommandoSocketFn = Box<(dyn Fn(Option<&Value>) -> Result<Value, Error> + Send + Sync + 'static)>;

/// Tooling to get a single control command socket where one can
/// register multiple commands dynamically.
///
/// You need to call `spawn()` to make the socket active.
pub struct CommandoSocket {
    socket: PathBuf,
    gid: Gid,
    commands: HashMap<String, CommandoSocketFn>,
}

impl CommandoSocket {
    pub fn new<P>(path: P, gid: Gid) -> Self
        where P: Into<PathBuf>,
    {
        CommandoSocket {
            socket: path.into(),
            gid,
            commands: HashMap::new(),
        }
    }

    /// Spawn the socket and consume self, meaning you cannot register commands anymore after
    /// calling this.
    pub fn spawn(self) -> Result<(), Error> {
        let control_future = create_control_socket(self.socket.to_owned(), self.gid, move |param| {
            let param = param
                .as_object()
                .ok_or_else(|| format_err!("unable to parse parameters (expected json object)"))?;

            let command = match param.get("command") {
                Some(Value::String(command)) => command.as_str(),
                None => bail!("no command"),
                _ => bail!("unable to parse command"),
            };

            if !self.commands.contains_key(command) {
                bail!("got unknown command '{}'", command);
            }

            match self.commands.get(command) {
                None => bail!("got unknown command '{}'", command),
                Some(handler) => {
                    let args = param.get("args"); //.unwrap_or(&Value::Null);
                    (handler)(args)
                },
            }
        })?;

        tokio::spawn(control_future);

        Ok(())
    }

    /// Register a new command with a callback.
    pub fn register_command<F>(
        &mut self,
        command: String,
        handler: F,
    ) -> Result<(), Error>
    where
        F: Fn(Option<&Value>) -> Result<Value, Error> + Send + Sync + 'static,
    {

        if self.commands.contains_key(&command) {
            bail!("command '{}' already exists!", command);
        }

        self.commands.insert(command, Box::new(handler));

        Ok(())
    }
}
