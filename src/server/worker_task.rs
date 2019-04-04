use failure::*;
use lazy_static::lazy_static;
use regex::Regex;
use chrono::Local;

use tokio::sync::oneshot;
use futures::*;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering, ATOMIC_USIZE_INIT};
use serde_json::{json, Value};

use crate::tools::{self, FileLogger};

macro_rules! PROXMOX_BACKUP_TASK_DIR { () => ("/var/log/proxmox-backup/tasks") }
macro_rules! PROXMOX_BACKUP_TASK_LOCK_FN { () => (concat!(PROXMOX_BACKUP_TASK_DIR!(), "/.active.lock")) }
macro_rules! PROXMOX_BACKUP_ACTIVE_TASK_FN { () => (concat!(PROXMOX_BACKUP_TASK_DIR!(), "/active")) }

lazy_static! {
    static ref WORKER_TASK_LIST: Mutex<HashMap<usize, Arc<WorkerTask>>> = Mutex::new(HashMap::new());
}

static WORKER_TASK_NEXT_ID: AtomicUsize = ATOMIC_USIZE_INIT;

#[derive(Debug, Clone)]
pub struct UPID {
    pub pid: libc::pid_t,
    pub pstart: u64,
    pub starttime: i64,
    pub task_id: usize,
    pub worker_type: String,
    pub worker_id: Option<String>,
    pub username: String,
    pub node: String,
}

impl std::str::FromStr for UPID {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {

        lazy_static! {
            static ref REGEX: Regex = Regex::new(concat!(
                r"^UPID:(?P<node>[a-zA-Z0-9]([a-zA-Z0-9\-]*[a-zA-Z0-9])?):(?P<pid>[0-9A-Fa-f]{8}):",
                r"(?P<pstart>[0-9A-Fa-f]{8,9}):(?P<task_id>[0-9A-Fa-f]{8,16}):(?P<starttime>[0-9A-Fa-f]{8}):",
                r"(?P<wtype>[^:\s]+):(?P<wid>[^:\s]*):(?P<username>[^:\s]+):$"
            )).unwrap();
        }

        if let Some(cap) = REGEX.captures(s) {

            return Ok(UPID {
                pid: i32::from_str_radix(&cap["pid"], 16).unwrap(),
                pstart: u64::from_str_radix(&cap["pstart"], 16).unwrap(),
                starttime: i64::from_str_radix(&cap["starttime"], 16).unwrap(),
                task_id: usize::from_str_radix(&cap["task_id"], 16).unwrap(),
                worker_type: cap["wtype"].to_string(),
                worker_id: if cap["wid"].is_empty() { None } else { Some(cap["wid"].to_string()) },
                username: cap["username"].to_string(),
                node: cap["node"].to_string(),
            });
        } else {
            bail!("unable to parse UPID '{}'", s);
        }

    }
}

impl std::fmt::Display for UPID {

    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {

        let wid = if let Some(ref id) = self.worker_id { id } else { "" };

        // Note: pstart can be > 32bit if uptime > 497 days, so this can result in
        // more that 8 characters for pstart

        write!(f, "UPID:{}:{:08X}:{:08X}:{:08X}:{:08X}:{}:{}:{}:",
               self.node, self.pid, self.pstart, self.task_id, self.starttime, self.worker_type, wid, self.username)
    }
}

#[derive(Debug)]
pub struct WorkerTaskInfo {
    upid: UPID,
    progress: f64, // 0..1
    abort_requested: bool,
}

pub fn running_worker_tasks() -> Vec<WorkerTaskInfo> {

    let mut list = vec![];

    for (_task_id, worker) in WORKER_TASK_LIST.lock().unwrap().iter() {
        let data = worker.data.lock().unwrap();
        let info = WorkerTaskInfo {
            upid: worker.upid.clone(),
            progress: data.progress,
            abort_requested: worker.abort_requested.load(Ordering::SeqCst),
        };
        list.push(info);
    }

    list
}

#[derive(Debug)]
pub struct WorkerTask {
    upid: UPID,
    data: Mutex<WorkerTaskData>,
    abort_requested: AtomicBool,
}

impl std::fmt::Display for WorkerTask {

    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.upid.fmt(f)
    }
}

#[derive(Debug)]
struct WorkerTaskData {
    logger: FileLogger,
    progress: f64, // 0..1
}

impl Drop for WorkerTask {

    fn drop(&mut self) {
        println!("unregister worker");
    }
}

impl WorkerTask {

    fn new(worker_type: &str, worker_id: Option<String>, username: &str, to_stdout: bool) -> Result<Arc<Self>, Error> {
        println!("register worker");

        let pid = unsafe { libc::getpid() };

        let task_id = WORKER_TASK_NEXT_ID.fetch_add(1, Ordering::SeqCst);

        let upid = UPID {
            pid,
            pstart: tools::procfs::read_proc_starttime(pid)?,
            starttime: Local::now().timestamp(),
            task_id,
            worker_type: worker_type.to_owned(),
            worker_id,
            username: username.to_owned(),
            node: tools::nodename().to_owned(),
        };

        let mut path = std::path::PathBuf::from(PROXMOX_BACKUP_TASK_DIR!());
        path.push(format!("{:02X}", upid.pstart % 256));

        let _ = std::fs::create_dir_all(&path); // ignore errors here

        path.push(upid.to_string());

        println!("FILE: {:?}", path);

        let logger = FileLogger::new(path, to_stdout)?;

        let worker = Arc::new(Self {
            upid: upid,
            abort_requested: AtomicBool::new(false),
            data: Mutex::new(WorkerTaskData {
                logger,
                progress: 0.0,
            }),
        });

        WORKER_TASK_LIST.lock().unwrap().insert(task_id, worker.clone());

        Ok(worker)
    }

    pub fn spawn<F, T>(worker_type: &str, worker_id: Option<String>, username: &str, to_stdout: bool, f: F) -> Result<(), Error>
        where F: Send + 'static + FnOnce(Arc<WorkerTask>) -> T,
              T: Send + 'static + Future<Item=(), Error=()>,
    {
        let worker = WorkerTask::new(worker_type, worker_id, username, to_stdout)?;
        let task_id = worker.upid.task_id;

        tokio::spawn(f(worker).then(move |_| {
            WORKER_TASK_LIST.lock().unwrap().remove(&task_id);
            Ok(())
        }));

        Ok(())
    }

    pub fn new_thread<F>(worker_type: &str, worker_id: Option<String>, username: &str, to_stdout: bool, f: F) -> Result<(), Error>
        where F: Send + 'static + FnOnce(Arc<WorkerTask>) -> ()
    {
        println!("register worker thread");

        let (p, c) = oneshot::channel::<()>();

        let worker = WorkerTask::new(worker_type, worker_id, username, to_stdout)?;
        let task_id = worker.upid.task_id;

        let _child = std::thread::spawn(move || {


            println!("start worker thread");
            f(worker);
            println!("end worker thread");

            WORKER_TASK_LIST.lock().unwrap().remove(&task_id);

            p.send(()).unwrap();
        });

        tokio::spawn(c.then(|_| Ok(())));

        Ok(())
    }

    pub fn log<S: AsRef<str>>(&self, msg: S) {
        let mut data = self.data.lock().unwrap();
        data.logger.log(msg);
    }

    pub fn progress(&self, progress: f64) {
        if progress >= 0.0 && progress <= 1.0 {
            let mut data = self.data.lock().unwrap();
            data.progress = progress;
        } else {
           // fixme:  log!("task '{}': ignoring strange value for progress '{}'", self.upid, progress);
        }
    }

    // request_abort
    pub fn request_abort(self) {
        self.abort_requested.store(true, Ordering::SeqCst);
    }

    pub fn abort_requested(&self) -> bool {
        self.abort_requested.load(Ordering::SeqCst)
    }

    pub fn fail_on_abort(&self) -> Result<(), Error> {
        if self.abort_requested() {
            bail!("task '{}': abort requested - aborting task", self.upid);
        }
        Ok(())
    }
}
