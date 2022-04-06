use anyhow::Error;
use lazy_static::lazy_static;
use std::sync::Mutex;

use futures::*;

use tokio::signal::unix::{signal, SignalKind};

use proxmox_async::broadcast_future::BroadcastData;

use crate::request_shutdown;

#[derive(PartialEq, Copy, Clone, Debug)]
enum ServerMode {
    Normal,
    Shutdown,
}

struct ServerState {
    mode: ServerMode,
    shutdown_listeners: BroadcastData<()>,
    last_worker_listeners: BroadcastData<()>,
    worker_count: usize,
    internal_task_count: usize,
    reload_request: bool,
}

lazy_static! {
    static ref SERVER_STATE: Mutex<ServerState> = Mutex::new(ServerState {
        mode: ServerMode::Normal,
        shutdown_listeners: BroadcastData::new(),
        last_worker_listeners: BroadcastData::new(),
        worker_count: 0,
        internal_task_count: 0,
        reload_request: false,
    });
}

/// Listen to ``SIGINT`` for server shutdown
///
/// This calls [request_shutdown] when receiving the signal.
pub fn catch_shutdown_signal() -> Result<(), Error> {
    let mut stream = signal(SignalKind::interrupt())?;

    let future = async move {
        while stream.recv().await.is_some() {
            log::info!("got shutdown request (SIGINT)");
            SERVER_STATE.lock().unwrap().reload_request = false;
            request_shutdown();
        }
    }
    .boxed();

    let abort_future = last_worker_future().map_err(|_| {});
    let task = futures::future::select(future, abort_future);

    tokio::spawn(task.map(|_| ()));

    Ok(())
}

/// Listen to ``SIGHUP`` for server reload
///
/// This calls [request_shutdown] when receiving the signal, and tries
/// to restart the server.
pub fn catch_reload_signal() -> Result<(), Error> {
    let mut stream = signal(SignalKind::hangup())?;

    let future = async move {
        while stream.recv().await.is_some() {
            log::info!("got reload request (SIGHUP)");
            SERVER_STATE.lock().unwrap().reload_request = true;
            crate::request_shutdown();
        }
    }
    .boxed();

    let abort_future = last_worker_future().map_err(|_| {});
    let task = futures::future::select(future, abort_future);

    tokio::spawn(task.map(|_| ()));

    Ok(())
}

pub(crate) fn is_reload_request() -> bool {
    let data = SERVER_STATE.lock().unwrap();

    data.mode == ServerMode::Shutdown && data.reload_request
}

pub(crate) fn server_shutdown() {
    let mut data = SERVER_STATE.lock().unwrap();

    log::info!("request_shutdown");

    data.mode = ServerMode::Shutdown;

    data.shutdown_listeners.notify_listeners(Ok(()));

    drop(data); // unlock

    check_last_worker();
}

/// Future to signal server shutdown
pub fn shutdown_future() -> impl Future<Output = ()> {
    let mut data = SERVER_STATE.lock().unwrap();
    data.shutdown_listeners.listen().map(|_| ())
}

/// Future to signal when last worker task finished
pub fn last_worker_future() -> impl Future<Output = Result<(), Error>> {
    let mut data = SERVER_STATE.lock().unwrap();
    data.last_worker_listeners.listen()
}

pub(crate) fn set_worker_count(count: usize) {
    SERVER_STATE.lock().unwrap().worker_count = count;

    check_last_worker();
}

pub(crate) fn check_last_worker() {
    let mut data = SERVER_STATE.lock().unwrap();

    if !(data.mode == ServerMode::Shutdown
        && data.worker_count == 0
        && data.internal_task_count == 0)
    {
        return;
    }

    data.last_worker_listeners.notify_listeners(Ok(()));
}

/// Spawns a tokio task that will be tracked for reload
/// and if it is finished, notify the [last_worker_future] if we
/// are in shutdown mode.
pub fn spawn_internal_task<T>(task: T)
where
    T: Future + Send + 'static,
    T::Output: Send + 'static,
{
    let mut data = SERVER_STATE.lock().unwrap();
    data.internal_task_count += 1;

    tokio::spawn(async move {
        let _ = tokio::spawn(task).await; // ignore errors

        {
            // drop mutex
            let mut data = SERVER_STATE.lock().unwrap();
            if data.internal_task_count > 0 {
                data.internal_task_count -= 1;
            }
        }

        check_last_worker();
    });
}
