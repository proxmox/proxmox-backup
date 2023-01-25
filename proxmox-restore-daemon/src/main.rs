///! Daemon binary to run inside a micro-VM for secure single file restore of disk images
use std::fs::File;
use std::io::prelude::*;
use std::os::unix::{
    io::{FromRawFd, RawFd},
    net,
};
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{bail, format_err, Error};
use lazy_static::lazy_static;
use log::{error, info};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use proxmox_router::RpcEnvironmentType;

use pbs_client::DEFAULT_VSOCK_PORT;
use proxmox_rest_server::{ApiConfig, RestServer};

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

fn init_disk_state() {
    info!("scanning all disks...");
    {
        let _disk_state = DISK_STATE.lock().unwrap();
    }

    info!("disk scan complete.")
}

/// This is expected to be run by 'proxmox-file-restore' within a mini-VM
fn main() -> Result<(), Error> {
    pbs_tools::setup_libc_malloc_opts();

    if !Path::new(VM_DETECT_FILE).exists() {
        bail!(
            "This binary is not supposed to be run manually, use 'proxmox-file-restore' instead."
        );
    }

    // don't have a real syslog (and no persistence), so use env_logger to print to a log file (via
    // stdout to a serial terminal attached by QEMU)
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .write_style(env_logger::WriteStyle::Never)
        .format_timestamp_millis()
        .init();

    info!("setup basic system environment...");
    setup_system_env().map_err(|err| format_err!("system environment setup failed: {}", err))?;

    proxmox_async::runtime::main(run())
}

/// ensure we have our /run dirs, system users and stuff like that setup
fn setup_system_env() -> Result<(), Error> {
    // the API may save some stuff there, e.g., the memcon tracking file
    // we do not care much, but it's way less headache to just create it
    std::fs::create_dir_all("/run/proxmox-backup")?;

    // we now ensure that all lock files are owned by the backup user, and as we reuse the
    // specialized REST module from pbs api/daemon we have some checks there for user/acl stuff
    // that gets locked, and thus needs the backup system user to work.
    std::fs::create_dir_all("/etc")?;
    let mut passwd = File::create("/etc/passwd")?;
    writeln!(passwd, "root:x:0:0:root:/root:/bin/sh")?;
    writeln!(
        passwd,
        "backup:x:34:34:backup:/var/backups:/usr/sbin/nologin"
    )?;

    let mut group = File::create("/etc/group")?;
    writeln!(group, "root:x:0:")?;
    writeln!(group, "backup:x:34:")?;

    Ok(())
}

async fn run() -> Result<(), Error> {
    watchdog_init();

    let init_future = async move {
        match tokio::time::timeout(
            std::time::Duration::from_secs(120),
            tokio::task::spawn_blocking(init_disk_state),
        )
        .await
        {
            Ok(res) => res.map_err(|err| format_err!("disk init failed: {}", err)),
            Err(_) => bail!("disk init timed out after 120 seconds"),
        }
    };

    let ticket =
        auth::read_ticket().map_err(|err| format_err!("reading ticket file failed: {}", err))?;

    let config = ApiConfig::new("", RpcEnvironmentType::PUBLIC)
        .default_api2_handler(&ROUTER)
        .index_handler_func(|_, _| auth::get_index())
        .auth_handler_func(move |h, m| Box::pin(auth::check_auth(Arc::clone(&ticket), h, m)));
    let rest_server = RestServer::new(config);

    let vsock_fd = get_vsock_fd()?;
    let connections = accept_vsock_connections(vsock_fd);
    let receiver_stream = ReceiverStream::new(connections);
    let acceptor = hyper::server::accept::from_stream(receiver_stream);

    let hyper_future = async move {
        hyper::Server::builder(acceptor)
            .serve(rest_server)
            .await
            .map_err(|err| format_err!("hyper finished with error: {}", err))
    };

    tokio::try_join!(init_future, hyper_future)?;

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
                    error!("error accepting vsock connection: {}", err);
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
    bind(sock_fd, &sock_addr)?;
    listen(sock_fd, MAX_PENDING)?;
    Ok(sock_fd)
}
