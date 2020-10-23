use anyhow::{bail, Error};

#[macro_use]
extern crate proxmox_backup;

extern crate tokio;
extern crate nix;

use proxmox::try_block;

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

    Ok(()).into()
}


#[test] #[ignore]
fn worker_task_abort() -> Result<(), Error> {

    server::create_task_log_dirs()?;

    use std::sync::{Arc, Mutex};

    let errmsg: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let errmsg1 = errmsg.clone();

    let mut rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {

        let init_result: Result<(), Error> = try_block!({
            server::create_task_control_socket()?;
            server::server_state_init()?;
            Ok(())
        });

        if let Err(err) = init_result {
            eprintln!("unable to start daemon - {}", err);
            return;
        }

        let errmsg = errmsg1.clone();
        let res = server::WorkerTask::new_thread(
            "garbage_collection",
            None,
            proxmox_backup::api2::types::Authid::root_auth_id().clone(),
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
                server::abort_worker_async(wid.parse::<server::UPID>().unwrap());
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
