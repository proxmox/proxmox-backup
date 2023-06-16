//! Helper to start a QEMU VM for single file restore.
use std::fs::{File, OpenOptions};
use std::io::prelude::*;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, format_err, Error};
use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt},
    time,
};

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

use proxmox_sys::fd::fd_change_cloexec;
use proxmox_sys::fs::{create_path, file_read_string, make_tmp_file, CreateOptions};
use proxmox_sys::logrotate::LogRotate;

use pbs_client::{VsockClient, DEFAULT_VSOCK_PORT};

use super::SnapRestoreDetails;
use crate::{backup_user, cpio};

const PBS_VM_NAME: &str = "pbs-restore-vm";
const MAX_CID_TRIES: u64 = 32;
pub const MAX_MEMORY_DIMM_SIZE: usize = 512;
const QMP_SOCKET_PREFIX: &str = "/run/proxmox-backup/file-restore-qmp-";

fn create_restore_log_dir() -> Result<String, Error> {
    let logpath = format!("{}/file-restore", pbs_buildcfg::PROXMOX_BACKUP_LOG_DIR);

    proxmox_lang::try_block!({
        let backup_user = backup_user()?;
        let opts = CreateOptions::new()
            .owner(backup_user.uid)
            .group(backup_user.gid);

        let opts_root = CreateOptions::new()
            .owner(nix::unistd::ROOT)
            .group(nix::unistd::Gid::from_raw(0));

        create_path(pbs_buildcfg::PROXMOX_BACKUP_LOG_DIR, None, Some(opts))?;
        create_path(&logpath, None, Some(opts_root))?;
        Ok(())
    })
    .map_err(|err: Error| format_err!("unable to create file-restore log dir - {err}"))?;

    Ok(logpath)
}

fn validate_img_existance(debug: bool) -> Result<(), Error> {
    let kernel = PathBuf::from(pbs_buildcfg::PROXMOX_BACKUP_KERNEL_FN);
    let initramfs = PathBuf::from(if debug {
        pbs_buildcfg::PROXMOX_BACKUP_INITRAMFS_DBG_FN
    } else {
        pbs_buildcfg::PROXMOX_BACKUP_INITRAMFS_FN
    });
    if !kernel.exists() || !initramfs.exists() {
        bail!("cannot run file-restore VM: package 'proxmox-backup-restore-image' is not (correctly) installed");
    }
    Ok(())
}

pub fn try_kill_vm(pid: i32) -> Result<(), Error> {
    let pid = Pid::from_raw(pid);
    if kill(pid, None).is_ok() {
        // process is running (and we could kill it), check if it is actually ours
        // (if it errors assume we raced with the process's death and ignore it)
        if let Ok(cmdline) = file_read_string(format!("/proc/{pid}/cmdline")) {
            if cmdline.split('\0').any(|a| a == PBS_VM_NAME) {
                // yes, it's ours, kill it brutally with SIGKILL, no reason to take
                // any chances - in this state it's most likely broken anyway
                if let Err(err) = kill(pid, Signal::SIGKILL) {
                    bail!("reaping broken VM (pid {pid}) with SIGKILL failed: {err}");
                }
            }
        }
    }

    Ok(())
}

async fn create_temp_initramfs(ticket: &str, debug: bool) -> Result<(File, String), Error> {
    use std::ffi::CString;
    use tokio::fs::File;

    let (tmp_file, tmp_path) =
        make_tmp_file("/tmp/file-restore-qemu.initramfs.tmp", CreateOptions::new())?;
    nix::unistd::unlink(&tmp_path)?;
    fd_change_cloexec(tmp_file.as_raw_fd(), false)?;

    let initramfs = if debug {
        pbs_buildcfg::PROXMOX_BACKUP_INITRAMFS_DBG_FN
    } else {
        pbs_buildcfg::PROXMOX_BACKUP_INITRAMFS_FN
    };

    let mut f = File::from_std(tmp_file);
    let mut base = File::open(initramfs).await?;

    tokio::io::copy(&mut base, &mut f).await?;

    let name = CString::new("ticket").unwrap();
    cpio::append_file(
        &mut f,
        ticket.as_bytes(),
        &name,
        cpio::Entry {
            mode: (libc::S_IFREG | 0o400) as u16,
            size: ticket.len() as u32,
            ..Default::default()
        },
    )
    .await?;
    cpio::append_trailer(&mut f).await?;

    let tmp_file = f.into_std().await;
    let path = format!("/dev/fd/{}", &tmp_file.as_raw_fd());

    Ok((tmp_file, path))
}

struct QMPSock {
    sock: tokio::io::BufStream<tokio::net::UnixStream>,
    initialized: bool,
}

impl QMPSock {
    /// Creates a new QMP socket connection. No active initialization is done until actual requests
    /// are made.
    pub async fn new(cid: i32) -> Result<Self, Error> {
        let path = format!("{QMP_SOCKET_PREFIX}{cid}.sock");
        let sock = tokio::io::BufStream::new(tokio::net::UnixStream::connect(path).await?);
        Ok(QMPSock {
            sock,
            initialized: false,
        })
    }

    /// Transparently serializes the QMP request and sends it out with `send_raw`
    pub async fn send(&mut self, request: serde_json::Value) -> Result<String, Error> {
        self.send_raw(&serde_json::to_string(&request)?).await
    }

    /// Send out raw (already serialized) QMP requests, handling initialization transparently
    pub async fn send_raw(&mut self, request: &str) -> Result<String, Error> {
        if !self.initialized {
            let _ = self.sock.read_line(&mut String::new()).await?; // initial qmp message
            self.do_send("{\"execute\":\"qmp_capabilities\"}\n").await?;
            self.initialized = true;
        }
        self.do_send(request).await
    }

    async fn do_send(&mut self, request: &str) -> Result<String, Error> {
        self.sock.write_all(request.as_bytes()).await?;
        self.sock.flush().await?;
        let mut buf = String::new();
        let _ = self.sock.read_line(&mut buf).await?;
        Ok(buf)
    }
}

pub(crate) async fn hotplug_memory(cid: i32, dimm_mb: usize) -> Result<(), Error> {
    if dimm_mb > MAX_MEMORY_DIMM_SIZE {
        bail!("cannot set to {dimm_mb}M, maximum is {MAX_MEMORY_DIMM_SIZE}M");
    }

    let mut qmp = QMPSock::new(cid).await?;

    qmp.send(json!({
        "execute": "object-add",
        "arguments": {
            "qom-type": "memory-backend-ram",
            "id": "mem0",
            "size": dimm_mb * 1024 * 1024,
        }
    }))
    .await?;

    qmp.send(json!({
        "execute": "device_add",
        "arguments": {
            "driver": "pc-dimm",
            "id": "dimm0",
            "memdev": "mem0",
        }
    }))
    .await?;

    Ok(())
}

pub fn debug_mode() -> bool {
    std::env::var("PBS_QEMU_DEBUG")
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

pub async fn start_vm(
    // u16 so we can do wrapping_add without going too high
    mut cid: u16,
    details: &SnapRestoreDetails,
    files: impl Iterator<Item = String>,
    ticket: &str,
) -> Result<(i32, i32), Error> {
    if std::env::var("PBS_PASSWORD").is_err() {
        bail!("environment variable PBS_PASSWORD has to be set for QEMU VM restore");
    }

    let debug = debug_mode();

    validate_img_existance(debug)?;

    let pid;
    let (mut pid_file, pid_path) =
        make_tmp_file("/tmp/file-restore-qemu.pid.tmp", CreateOptions::new())?;
    nix::unistd::unlink(&pid_path)?;
    fd_change_cloexec(pid_file.as_raw_fd(), false)?;

    let (_ramfs_pid, ramfs_path) = create_temp_initramfs(ticket, debug).await?;

    let logpath = create_restore_log_dir()?;
    let logfile = &format!("{logpath}/qemu.log");
    let mut logrotate = LogRotate::new(logfile, false, Some(16), None)?;

    if let Err(err) = logrotate.do_rotate() {
        log::warn!("warning: logrotate for QEMU log file failed - {err}");
    }

    let mut logfd = OpenOptions::new()
        .append(true)
        .create_new(true)
        .open(logfile)?;
    fd_change_cloexec(logfd.as_raw_fd(), false)?;

    // preface log file with start timestamp so one can see how long QEMU took to start
    writeln!(logfd, "[{}] PBS file restore VM log", {
        let now = proxmox_time::epoch_i64();
        proxmox_time::epoch_to_rfc3339(now)?
    },)?;

    let base_args = [
        "-chardev",
        &format!(
            "file,id=log,path=/dev/null,logfile=/dev/fd/{},logappend=on",
            logfd.as_raw_fd()
        ),
        "-serial",
        "chardev:log",
        "-vnc",
        "none",
        "-enable-kvm",
        "-kernel",
        pbs_buildcfg::PROXMOX_BACKUP_KERNEL_FN,
        "-initrd",
        &ramfs_path,
        "-append",
        // NOTE: ZFS requires that the ARC can at least grow to the max transaction size of 64MB
        // also: setting any of min/max to zero will rather do the opposite of what one wants here
        &format!(
            "{} panic=1 zfs.zfs_arc_min=33554432 zfs.zfs_arc_max=67108864 memhp_default_state=online_kernel",
            if debug { "debug" } else { "quiet" }
        ),
        "-daemonize",
        "-pidfile",
        &format!("/dev/fd/{}", pid_file.as_raw_fd()),
        "-name",
        PBS_VM_NAME,
    ];

    // Generate drive arguments for all fidx files in backup snapshot
    let mut drives = Vec::new();
    let mut id = 0;
    for file in files {
        if !file.ends_with(".img.fidx") {
            continue;
        }
        drives.push("-drive".to_owned());
        let keyfile = if let Some(ref keyfile) = details.keyfile {
            format!(",,keyfile={keyfile}")
        } else {
            "".to_owned()
        };
        let namespace = if details.namespace.is_root() {
            String::new()
        } else {
            format!(",,namespace={}", details.namespace)
        };
        drives.push(format!(
            "file=pbs:repository={}{},,snapshot={},,archive={}{},read-only=on,if=none,id=drive{}",
            details.repo, namespace, details.snapshot, file, keyfile, id
        ));

        // a PCI bus can only support 32 devices, so add a new one every 32
        let bus = (id / 32) + 2;
        if id % 32 == 0 {
            drives.push("-device".to_owned());
            drives.push(format!("pci-bridge,id=bridge{bus},chassis_nr={bus}"));
        }

        drives.push("-device".to_owned());
        // drive serial is used by VM to map .fidx files to /dev paths
        let serial = file.strip_suffix(".img.fidx").unwrap_or(&file);
        drives.push(format!(
            "virtio-blk-pci,drive=drive{id},serial={serial},bus=bridge{bus}"
        ));
        id += 1;
    }

    let ram = if debug {
        1024
    } else {
        // add more RAM if many drives are given
        match id {
            f if f < 10 => 192,
            f if f < 20 => 256,
            _ => 384,
        }
    };

    // Try starting QEMU in a loop to retry if we fail because of a bad 'cid' value
    let mut attempts = 0;
    loop {
        let mut qemu_cmd = std::process::Command::new("qemu-system-x86_64");
        qemu_cmd.args(base_args.iter());
        qemu_cmd.arg("-m");
        qemu_cmd.arg(format!(
            "{ram}M,slots=1,maxmem={}M",
            MAX_MEMORY_DIMM_SIZE + ram
        ));
        qemu_cmd.args(&drives);
        qemu_cmd.arg("-device");
        qemu_cmd.arg(format!(
            "vhost-vsock-pci,guest-cid={},disable-legacy=on",
            cid
        ));
        qemu_cmd.arg("-chardev");
        qemu_cmd.arg(format!(
            "socket,id=qmp,path={}{}.sock,server=on,wait=off",
            QMP_SOCKET_PREFIX, cid
        ));
        qemu_cmd.arg("-mon");
        qemu_cmd.arg("chardev=qmp,mode=control");

        if debug {
            let debug_args = [
                "-chardev",
                &format!(
                    "socket,id=debugser,path=/run/proxmox-backup/file-restore-serial-{}.sock,server=on,wait=off",
                    cid
                ),
                "-serial",
                "chardev:debugser",
            ];
            qemu_cmd.args(debug_args.iter());
        }

        qemu_cmd.stdout(std::process::Stdio::null());
        qemu_cmd.stderr(std::process::Stdio::piped());

        let res = tokio::task::block_in_place(|| qemu_cmd.spawn()?.wait_with_output())?;

        if res.status.success() {
            // at this point QEMU is already daemonized and running, so if anything fails we
            // technically leave behind a zombie-VM... this shouldn't matter, as it will stop
            // itself soon enough (timer), and the following operations are unlikely to fail
            let mut pidstr = String::new();
            pid_file.read_to_string(&mut pidstr)?;
            pid = pidstr.trim_end().parse().map_err(|err| {
                format_err!("cannot parse PID returned by QEMU ('{pidstr}'): {err}")
            })?;
            break;
        } else {
            let out = String::from_utf8_lossy(&res.stderr);
            if out.contains("unable to set guest cid: Address already in use") {
                attempts += 1;
                if attempts >= MAX_CID_TRIES {
                    bail!("CID '{cid}' in use, but max attempts reached, aborting");
                }
                // CID in use, try next higher one
                log::info!("CID '{cid}' in use by other VM, attempting next one");
                // skip special-meaning low values
                cid = cid.wrapping_add(1).max(10);
            } else {
                log::error!("{out}");
                bail!("Starting VM failed. See output above for more information.");
            }
        }
    }

    // QEMU has started successfully, now wait for virtio socket to become ready
    let pid_t = Pid::from_raw(pid);

    let start_poll = Instant::now();
    let mut round = 1;
    loop {
        let client = VsockClient::new(cid as i32, DEFAULT_VSOCK_PORT, Some(ticket.to_owned()));
        if let Ok(Ok(_)) =
            time::timeout(Duration::from_secs(2), client.get("api2/json/status", None)).await
        {
            log::debug!(
                "Connect to '/run/proxmox-backup/file-restore-serial-{cid}.sock' for shell access"
            );
            return Ok((pid, cid as i32));
        }
        if kill(pid_t, None).is_err() {
            // check if QEMU process exited in between
            bail!("VM exited before connection could be established");
        }
        if Instant::now().duration_since(start_poll) > Duration::from_secs(25) {
            break;
        }
        time::sleep(Duration::from_millis(round * 25)).await;
        round += 1;
    }

    // start failed
    if let Err(err) = try_kill_vm(pid) {
        log::error!("killing failed VM failed: {err}");
    }
    bail!("starting VM timed out");
}
