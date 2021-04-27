//! Helper to start a QEMU VM for single file restore.
use std::fs::{File, OpenOptions};
use std::io::prelude::*;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, format_err, Error};
use tokio::time;

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

use proxmox::tools::{
    fd::Fd,
    fs::{create_path, file_read_string, make_tmp_file, CreateOptions},
};

use proxmox_backup::backup::backup_user;
use proxmox_backup::client::{VsockClient, DEFAULT_VSOCK_PORT};
use proxmox_backup::{buildcfg, tools};

use super::SnapRestoreDetails;

const PBS_VM_NAME: &str = "pbs-restore-vm";
const MAX_CID_TRIES: u64 = 32;

fn create_restore_log_dir() -> Result<String, Error> {
    let logpath = format!("{}/file-restore", buildcfg::PROXMOX_BACKUP_LOG_DIR);

    proxmox::try_block!({
        let backup_user = backup_user()?;
        let opts = CreateOptions::new()
            .owner(backup_user.uid)
            .group(backup_user.gid);

        let opts_root = CreateOptions::new()
            .owner(nix::unistd::ROOT)
            .group(nix::unistd::Gid::from_raw(0));

        create_path(buildcfg::PROXMOX_BACKUP_LOG_DIR, None, Some(opts))?;
        create_path(&logpath, None, Some(opts_root))?;
        Ok(())
    })
    .map_err(|err: Error| format_err!("unable to create file-restore log dir - {}", err))?;

    Ok(logpath)
}

fn validate_img_existance() -> Result<(), Error> {
    let kernel = PathBuf::from(buildcfg::PROXMOX_BACKUP_KERNEL_FN);
    let initramfs = PathBuf::from(buildcfg::PROXMOX_BACKUP_INITRAMFS_FN);
    if !kernel.exists() || !initramfs.exists() {
        bail!("cannot run file-restore VM: package 'proxmox-backup-restore-image' is not (correctly) installed");
    }
    Ok(())
}

fn try_kill_vm(pid: i32) -> Result<(), Error> {
    let pid = Pid::from_raw(pid);
    if let Ok(()) = kill(pid, None) {
        // process is running (and we could kill it), check if it is actually ours
        // (if it errors assume we raced with the process's death and ignore it)
        if let Ok(cmdline) = file_read_string(format!("/proc/{}/cmdline", pid)) {
            if cmdline.split('\0').any(|a| a == PBS_VM_NAME) {
                // yes, it's ours, kill it brutally with SIGKILL, no reason to take
                // any chances - in this state it's most likely broken anyway
                if let Err(err) = kill(pid, Signal::SIGKILL) {
                    bail!(
                        "reaping broken VM (pid {}) with SIGKILL failed: {}",
                        pid,
                        err
                    );
                }
            }
        }
    }

    Ok(())
}

async fn create_temp_initramfs(ticket: &str) -> Result<(Fd, String), Error> {
    use std::ffi::CString;
    use tokio::fs::File;

    let (tmp_fd, tmp_path) =
        make_tmp_file("/tmp/file-restore-qemu.initramfs.tmp", CreateOptions::new())?;
    nix::unistd::unlink(&tmp_path)?;
    tools::fd_change_cloexec(tmp_fd.0, false)?;

    let mut f = File::from_std(unsafe { std::fs::File::from_raw_fd(tmp_fd.0) });
    let mut base = File::open(buildcfg::PROXMOX_BACKUP_INITRAMFS_FN).await?;

    tokio::io::copy(&mut base, &mut f).await?;

    let name = CString::new("ticket").unwrap();
    tools::cpio::append_file(
        &mut f,
        ticket.as_bytes(),
        &name,
        0,
        (libc::S_IFREG | 0o400) as u16,
        0,
        0,
        0,
        ticket.len() as u32,
    )
    .await?;
    tools::cpio::append_trailer(&mut f).await?;

    // forget the tokio file, we close the file descriptor via the returned Fd
    std::mem::forget(f);

    let path = format!("/dev/fd/{}", &tmp_fd.0);
    Ok((tmp_fd, path))
}

pub async fn start_vm(
    // u16 so we can do wrapping_add without going too high
    mut cid: u16,
    details: &SnapRestoreDetails,
    files: impl Iterator<Item = String>,
    ticket: &str,
) -> Result<(i32, i32), Error> {
    validate_img_existance()?;

    if let Err(_) = std::env::var("PBS_PASSWORD") {
        bail!("environment variable PBS_PASSWORD has to be set for QEMU VM restore");
    }

    let pid;
    let (pid_fd, pid_path) = make_tmp_file("/tmp/file-restore-qemu.pid.tmp", CreateOptions::new())?;
    nix::unistd::unlink(&pid_path)?;
    tools::fd_change_cloexec(pid_fd.0, false)?;

    let (_ramfs_pid, ramfs_path) = create_temp_initramfs(ticket).await?;

    let logpath = create_restore_log_dir()?;
    let logfile = &format!("{}/qemu.log", logpath);
    let mut logrotate = tools::logrotate::LogRotate::new(logfile, false)
        .ok_or_else(|| format_err!("could not get QEMU log file names"))?;

    if let Err(err) = logrotate.do_rotate(CreateOptions::default(), Some(16)) {
        eprintln!("warning: logrotate for QEMU log file failed - {}", err);
    }

    let mut logfd = OpenOptions::new()
        .append(true)
        .create_new(true)
        .open(logfile)?;
    tools::fd_change_cloexec(logfd.as_raw_fd(), false)?;

    // preface log file with start timestamp so one can see how long QEMU took to start
    writeln!(logfd, "[{}] PBS file restore VM log", {
        let now = proxmox::tools::time::epoch_i64();
        proxmox::tools::time::epoch_to_rfc3339(now)?
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
        "-m",
        "128",
        "-kernel",
        buildcfg::PROXMOX_BACKUP_KERNEL_FN,
        "-initrd",
        &ramfs_path,
        "-append",
        "quiet panic=1",
        "-daemonize",
        "-pidfile",
        &format!("/dev/fd/{}", pid_fd.as_raw_fd()),
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
            format!(",,keyfile={}", keyfile)
        } else {
            "".to_owned()
        };
        drives.push(format!(
            "file=pbs:repository={},,snapshot={},,archive={}{},read-only=on,if=none,id=drive{}",
            details.repo, details.snapshot, file, keyfile, id
        ));
        drives.push("-device".to_owned());
        // drive serial is used by VM to map .fidx files to /dev paths
        drives.push(format!("virtio-blk-pci,drive=drive{},serial={}", id, file));
        id += 1;
    }

    // Try starting QEMU in a loop to retry if we fail because of a bad 'cid' value
    let mut attempts = 0;
    loop {
        let mut qemu_cmd = std::process::Command::new("qemu-system-x86_64");
        qemu_cmd.args(base_args.iter());
        qemu_cmd.args(&drives);
        qemu_cmd.arg("-device");
        qemu_cmd.arg(format!(
            "vhost-vsock-pci,guest-cid={},disable-legacy=on",
            cid
        ));

        qemu_cmd.stdout(std::process::Stdio::null());
        qemu_cmd.stderr(std::process::Stdio::piped());

        let res = tokio::task::block_in_place(|| qemu_cmd.spawn()?.wait_with_output())?;

        if res.status.success() {
            // at this point QEMU is already daemonized and running, so if anything fails we
            // technically leave behind a zombie-VM... this shouldn't matter, as it will stop
            // itself soon enough (timer), and the following operations are unlikely to fail
            let mut pid_file = unsafe { File::from_raw_fd(pid_fd.as_raw_fd()) };
            std::mem::forget(pid_fd); // FD ownership is now in pid_fd/File
            let mut pidstr = String::new();
            pid_file.read_to_string(&mut pidstr)?;
            pid = pidstr.trim_end().parse().map_err(|err| {
                format_err!("cannot parse PID returned by QEMU ('{}'): {}", &pidstr, err)
            })?;
            break;
        } else {
            let out = String::from_utf8_lossy(&res.stderr);
            if out.contains("unable to set guest cid: Address already in use") {
                attempts += 1;
                if attempts >= MAX_CID_TRIES {
                    bail!("CID '{}' in use, but max attempts reached, aborting", cid);
                }
                // CID in use, try next higher one
                eprintln!("CID '{}' in use by other VM, attempting next one", cid);
                // skip special-meaning low values
                cid = cid.wrapping_add(1).max(10);
            } else {
                eprint!("{}", out);
                bail!("Starting VM failed. See output above for more information.");
            }
        }
    }

    // QEMU has started successfully, now wait for virtio socket to become ready
    let pid_t = Pid::from_raw(pid);
    for _ in 0..60 {
        let client = VsockClient::new(cid as i32, DEFAULT_VSOCK_PORT, Some(ticket.to_owned()));
        if let Ok(Ok(_)) =
            time::timeout(Duration::from_secs(2), client.get("api2/json/status", None)).await
        {
            return Ok((pid, cid as i32));
        }
        if kill(pid_t, None).is_err() {
            // QEMU exited
            bail!("VM exited before connection could be established");
        }
        time::sleep(Duration::from_millis(200)).await;
    }

    // start failed
    if let Err(err) = try_kill_vm(pid) {
        eprintln!("killing failed VM failed: {}", err);
    }
    bail!("starting VM timed out");
}
