///! Daemon binary to run inside a micro-VM for secure single file restore of disk images
use anyhow::{bail, format_err, Error};
use lazy_static::lazy_static;
use log::{info, error};

use std::os::unix::{
    io::{FromRawFd, RawFd},
    net,
};
use std::path::Path;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use proxmox::api::RpcEnvironmentType;
use proxmox_backup::client::DEFAULT_VSOCK_PORT;
use proxmox_backup::server::{rest::*, ApiConfig};

mod proxmox_restore_daemon;
use proxmox_restore_daemon::*;

/// Maximum amount of pending requests. If saturated, virtio-vsock returns ETIMEDOUT immediately.
/// We should never have more than a few requests in queue, so use a low number.
pub const MAX_PENDING: usize = 32;

/// Will be present in base initramfs
pub const VM_DETECT_FILE: &str = "/restore-vm-marker";

lazy_static! {
    /// The current disks state. Use for accessing data on the attached snapshots.
    pub static ref DISK_STATE: Arc<Mutex<DiskState>> = {
        Arc::new(Mutex::new(DiskState::scan().unwrap()))
    };
}

/// This is expected to be run by 'proxmox-file-restore' within a mini-VM
fn main() -> Result<(), Error> {
    if !Path::new(VM_DETECT_FILE).exists() {
        bail!(
            "This binary is not supposed to be run manually, use 'proxmox-file-restore' instead."
        );
    }

    // don't have a real syslog (and no persistance), so use env_logger to print to a log file (via
    // stdout to a serial terminal attached by QEMU)
    env_logger::from_env(env_logger::Env::default().default_filter_or("info"))
        .write_style(env_logger::WriteStyle::Never)
        .init();

    // the API may save some stuff there, e.g., the memcon tracking file
    // we do not care much, but it's way less headache to just create it
    std::fs::create_dir_all("/run/proxmox-backup")?;

    // scan all attached disks now, before starting the API
    // this will panic and stop the VM if anything goes wrong
    info!("scanning all disks...");
    {
        let _disk_state = DISK_STATE.lock().unwrap();
    }

    info!("disk scan complete, starting main runtime...");

    proxmox_backup::tools::runtime::main(run())
}

async fn run() -> Result<(), Error> {
    watchdog_init();

    let auth_config = Arc::new(
        auth::ticket_auth().map_err(|err| format_err!("reading ticket file failed: {}", err))?,
    );
    let config = ApiConfig::new("", &ROUTER, RpcEnvironmentType::PUBLIC, auth_config)?;
    let rest_server = RestServer::new(config);

    let vsock_fd = get_vsock_fd()?;
    let connections = accept_vsock_connections(vsock_fd);
    let receiver_stream = ReceiverStream::new(connections);
    let acceptor = hyper::server::accept::from_stream(receiver_stream);

    hyper::Server::builder(acceptor).serve(rest_server).await?;

    bail!("hyper server exited");
}

fn accept_vsock_connections(
    vsock_fd: RawFd,
) -> mpsc::Receiver<Result<tokio::net::UnixStream, Error>> {
    use nix::sys::socket::*;
    let (sender, receiver) = mpsc::channel(MAX_PENDING);

    tokio::spawn(async move {
        loop {
            let stream: Result<tokio::net::UnixStream, Error> = tokio::task::block_in_place(|| {
                // we need to accept manually, as UnixListener aborts if socket type != AF_UNIX ...
                let client_fd = accept(vsock_fd)?;
                let stream = unsafe { net::UnixStream::from_raw_fd(client_fd) };
                stream.set_nonblocking(true)?;
                tokio::net::UnixStream::from_std(stream).map_err(|err| err.into())
            });

            match stream {
                Ok(stream) => {
                    if sender.send(Ok(stream)).await.is_err() {
                        error!("connection accept channel was closed");
                    }
                }
                Err(err) => {
                    error!("error accepting vsock connetion: {}", err);
                }
            }
        }
    });

    receiver
}

fn get_vsock_fd() -> Result<RawFd, Error> {
    use nix::sys::socket::*;
    let sock_fd = socket(
        AddressFamily::Vsock,
        SockType::Stream,
        SockFlag::empty(),
        None,
    )?;
    let sock_addr = VsockAddr::new(libc::VMADDR_CID_ANY, DEFAULT_VSOCK_PORT as u32);
    bind(sock_fd, &SockAddr::Vsock(sock_addr))?;
    listen(sock_fd, MAX_PENDING)?;
    Ok(sock_fd)
}
