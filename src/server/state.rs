use failure::*;
use lazy_static::lazy_static;
use std::sync::Mutex;

use futures::*;
use futures::stream::Stream;

use tokio::sync::oneshot;
use tokio_signal::unix::{Signal, SIGHUP, SIGINT};

use crate::tools;

#[derive(PartialEq, Copy, Clone, Debug)]
pub enum ServerMode {
    Normal,
    Shutdown,
}

pub struct ServerState {
    pub mode: ServerMode,
    pub shutdown_listeners: Vec<oneshot::Sender<()>>,
    pub last_worker_listeners: Vec<oneshot::Sender<()>>,
    pub worker_count: usize,
    pub reload_request: bool,
}


lazy_static! {
    static ref SERVER_STATE: Mutex<ServerState> = Mutex::new(ServerState {
        mode: ServerMode::Normal,
        shutdown_listeners: vec![],
        last_worker_listeners: vec![],
        worker_count: 0,
        reload_request: false,
    });
}

pub fn server_state_init() -> Result<(), Error> {

    let stream = Signal::new(SIGINT).flatten_stream();

    let future = stream.for_each(|_| {
        println!("got shutdown request (SIGINT)");
        SERVER_STATE.lock().unwrap().reload_request = false;
        tools::request_shutdown();
        Ok(())
    }).map_err(|_| {});

    let abort_future = last_worker_future().map_err(|_| {});
    let task = future.select(abort_future);

    tokio::spawn(task.map(|_| {}).map_err(|_| {}));

    let stream = Signal::new(SIGHUP).flatten_stream();

    let future = stream.for_each(|_| {
        println!("got reload request (SIGHUP)");
        SERVER_STATE.lock().unwrap().reload_request = true;
        tools::request_shutdown();
        Ok(())
    }).map_err(|_| {});

    let abort_future = last_worker_future().map_err(|_| {});
    let task = future.select(abort_future);

    tokio::spawn(task.map(|_| {}).map_err(|_| {}));

    Ok(())
}

pub fn is_reload_request() -> bool {
    let data = SERVER_STATE.lock().unwrap();

    if data.mode == ServerMode::Shutdown && data.reload_request {
        true
    } else {
        false
    }
}

pub fn server_shutdown() {
    let mut data = SERVER_STATE.lock().unwrap();

    println!("SET SHUTDOWN MODE");

    data.mode = ServerMode::Shutdown;

    notify_listeners(&mut data.shutdown_listeners);

    drop(data); // unlock

    check_last_worker();
}

pub fn shutdown_future() -> oneshot::Receiver<()> {
    let (tx, rx) = oneshot::channel::<()>();

    let mut data = SERVER_STATE.lock().unwrap();
    match data.mode {
        ServerMode::Normal => { data.shutdown_listeners.push(tx); },
        ServerMode::Shutdown =>  { let _ = tx.send(()); },
    }

    rx
}

pub fn last_worker_future() -> oneshot::Receiver<()> {
    let (tx, rx) = oneshot::channel::<()>();

    let mut data = SERVER_STATE.lock().unwrap();
    if data.mode == ServerMode::Shutdown && data.worker_count == 0 {
        let _ = tx.send(());
    } else {
        data.last_worker_listeners.push(tx);
    }

    rx
}

pub fn set_worker_count(count: usize) {
    let mut data = SERVER_STATE.lock().unwrap();
    data.worker_count = count;

    if !(data.mode == ServerMode::Shutdown && data.worker_count == 0) { return; }

    notify_listeners(&mut data.last_worker_listeners);
}


pub fn check_last_worker() {

    let mut data = SERVER_STATE.lock().unwrap();

    if !(data.mode == ServerMode::Shutdown && data.worker_count == 0) { return; }

    notify_listeners(&mut data.last_worker_listeners);
}

fn notify_listeners(list: &mut Vec<oneshot::Sender<()>>) {
    loop {
        match list.pop() {
            None => { break; },
            Some(ch) => {
                println!("SEND ABORT");
                if let Err(_) = ch.send(()) {
                    eprintln!("SEND ABORT failed");
                }
            },
        }
    }
}
