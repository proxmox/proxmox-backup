use anyhow::{bail, Error};

extern crate proxmox_backup;

extern crate nix;
extern crate tokio;

use proxmox_lang::try_block;
use proxmox_sys::fs::CreateOptions;
use proxmox_sys::{task_log, WorkerTaskContext};

use pbs_api_types::{Authid, UPID};

use proxmox_rest_server::{CommandSocket, WorkerTask};

fn garbage_collection(worker: &WorkerTask) -> Result<(), Error> {
    task_log!(worker, "start garbage collection");

    for i in 0..50 {
        worker.check_abort()?;

        task_log!(worker, "progress {}", i);

        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    task_log!(worker, "end garbage collection");

    Ok(())
}

#[test]
#[ignore]
fn worker_task_abort() -> Result<(), Error> {
    let uid = nix::unistd::Uid::current();
    let gid = nix::unistd::Gid::current();

    let file_opts = CreateOptions::new().owner(uid).group(gid);
    proxmox_rest_server::init_worker_tasks("./target/tasklogtestdir".into(), file_opts)?;

    use std::sync::{Arc, Mutex};

    let errmsg: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let errmsg1 = errmsg.clone();

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        let mut commando_sock = CommandSocket::new(
            proxmox_rest_server::our_ctrl_sock(),
            nix::unistd::Gid::current(),
        );

        let init_result: Result<(), Error> = try_block!({
            proxmox_rest_server::register_task_control_commands(&mut commando_sock)?;
            Ok(())
        });

        if let Err(err) = init_result {
            eprintln!("unable to start daemon - {}", err);
            return;
        }

        if let Err(err) = commando_sock.spawn() {
            eprintln!("unable to spawn command socket - {}", err);
            return;
        }

        let errmsg = errmsg1.clone();
        let res = WorkerTask::new_thread(
            "garbage_collection",
            None,
            Authid::root_auth_id().to_string(),
            true,
            move |worker| {
                println!("WORKER {}", worker);

                let result = garbage_collection(&worker);
                proxmox_rest_server::request_shutdown();

                if let Err(err) = result {
                    println!("got expected error: {}", err);
                } else {
                    let mut data = errmsg.lock().unwrap();
                    *data = Some(String::from(
                        "thread finished - seems abort did not work as expected",
                    ));
                }

                Ok(())
            },
        );

        match res {
            Err(err) => {
                println!("unable to start worker - {}", err);
            }
            Ok(wid) => {
                println!("WORKER: {}", wid);
                proxmox_rest_server::abort_worker_nowait(wid.parse::<UPID>().unwrap());
                proxmox_rest_server::wait_for_local_worker(&wid)
                    .await
                    .unwrap();
            }
        }
    });

    let data = errmsg.lock().unwrap();
    if let Some(ref err) = *data {
        bail!("Error: {}", err)
    }

    Ok(())
}
