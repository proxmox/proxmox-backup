use std::thread;
use std::sync::{Arc, Mutex};
use failure::*;
use tokio::prelude::*;

struct StorageOperation {
    state: Arc<Mutex<bool>>,
    running: bool,
}

impl StorageOperation {

    fn new() -> Self {
        StorageOperation { state: Arc::new(Mutex::new(false)), running: false }
    }

    fn run(&mut self, task: task::Task) {

        let state = self.state.clone();

        thread::spawn(move || {
            println!("state {}", state.lock().unwrap());
            println!("Starting Asnyc worker thread (delay 1 second)");
            thread::sleep(::std::time::Duration::from_secs(1));
            println!("End Asnyc worker thread");
            *state.lock().unwrap() = true;
            task.notify();
        });
    }
}

impl Future for StorageOperation {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if *self.state.lock().unwrap() != true {
            println!("not ready - parking the task.");

            if !self.running {
                println!("starting storage thread");
                self.run(task::current());
                self.running = true;
            }

            Ok(Async::NotReady)
        } else {
            println!("storage thread ready - task will complete.");
            Ok(Async::Ready(()))
        }
    }
}


#[test]
fn test_storage_future()
{

    let op = StorageOperation::new();
    hyper::rt::run(op.map_err(|e| {
        println!("Got Error: {}", e);
        ()
    }));
}
