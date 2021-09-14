use anyhow::{bail, Error};

#[macro_use]
extern crate proxmox_backup;

extern crate tokio;
extern crate nix;

use proxmox::try_block;

use pbs_api_types::{Authid, UPID};

use proxmox_backup::server;
use proxmox_backup::tools;

fn garbage_collection(worker: &server::WorkerTask) -> Result<(), Error> {

    worker.log("start garbage collection");

    for i in 0..50 {
        worker.fail_on_abort()?;

        flog!(worker, "progress {}", i);

        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    worker.log("end garbage collection");

    Ok(())
}


#[test]
#[ignore]
fn worker_task_abort() -> Result<(), Error> {

    server::create_task_log_dirs()?;

    use std::sync::{Arc, Mutex};

    let errmsg: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let errmsg1 = errmsg.clone();

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {

        let mut commando_sock = server::CommandoSocket::new(server::our_ctrl_sock());

        let init_result: Result<(), Error> = try_block!({
            server::register_task_control_commands(&mut commando_sock)?;
            server::server_state_init()?;
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
        let res = server::WorkerTask::new_thread(
            "garbage_collection",
            None,
            Authid::root_auth_id().clone(),
            true,
            move |worker| {
                println!("WORKER {}", worker);

                let result = garbage_collection(&worker);
                tools::request_shutdown();

                if let Err(err) = result {
                    println!("got expected error: {}", err);
                } else {
                    let mut data = errmsg.lock().unwrap();
                    *data = Some(String::from("thread finished - seems abort did not work as expected"));
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
                server::abort_worker_async(wid.parse::<UPID>().unwrap());
                server::wait_for_local_worker(&wid).await.unwrap();
             }
        }
    });

    let data = errmsg.lock().unwrap();
    match *data {
        Some(ref err) => bail!("Error: {}", err),
        None => {},
    }

    Ok(())
}
