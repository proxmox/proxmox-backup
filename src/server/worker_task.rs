use failure::*;
use lazy_static::lazy_static;
use regex::Regex;
use chrono::Local;

use tokio::sync::oneshot;
use futures::*;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering, ATOMIC_USIZE_INIT};
use std::io::{BufRead, BufReader};
use std::fs::File;

use crate::tools::{self, FileLogger};

macro_rules! PROXMOX_BACKUP_LOG_DIR { () => ("/var/log/proxmox-backup") }
macro_rules! PROXMOX_BACKUP_TASK_DIR { () => (concat!( PROXMOX_BACKUP_LOG_DIR!(), "/tasks")) }
macro_rules! PROXMOX_BACKUP_TASK_LOCK_FN { () => (concat!(PROXMOX_BACKUP_TASK_DIR!(), "/.active.lock")) }
macro_rules! PROXMOX_BACKUP_ACTIVE_TASK_FN { () => (concat!(PROXMOX_BACKUP_TASK_DIR!(), "/active")) }

lazy_static! {
    static ref WORKER_TASK_LIST: Mutex<HashMap<usize, Arc<WorkerTask>>> = Mutex::new(HashMap::new());
}

static WORKER_TASK_NEXT_ID: AtomicUsize = ATOMIC_USIZE_INIT;

/// Unique Process/Task Identifier
///
/// We use this to uniquely identify worker task. UPIDs have a short
/// string repesentaion, which gives additional information about the
/// type of the task. for example:
/// ```text
/// UPID:{node}:{pid}:{pstart}:{task_id}:{starttime}:{worker_type}:{worker_id}:{username}:
/// UPID:elsa:00004F37:0039E469:00000000:5CA78B83:garbage_collection::root@pam:
/// ```
/// Please note that we use tokio, so a single thread can run multiple
/// tasks.
#[derive(Debug, Clone)]
pub struct UPID {
    /// The Unix PID
    pub pid: libc::pid_t,
    /// The Unix process start time from `/proc/pid/stat`
    pub pstart: u64,
    /// The task start time (Epoch)
    pub starttime: i64,
    /// The task ID (inside the process/thread)
    pub task_id: usize,
    /// Worker type (arbitrary ASCII string)
    pub worker_type: String,
    /// Worker ID (arbitrary ASCII string)
    pub worker_id: Option<String>,
    /// The user who started the task
    pub username: String,
    /// The node name.
    pub node: String,
}

impl UPID {

    /// Returns the absolute path to the task log file
    pub fn log_path(&self) -> std::path::PathBuf {
        let mut path = std::path::PathBuf::from(PROXMOX_BACKUP_TASK_DIR!());
        path.push(format!("{:02X}", self.pstart % 256));
        path.push(self.to_string());
        path
    }

    /// Test if the task is still running
    pub fn is_active(&self) -> bool {

        lazy_static! {
            static ref MY_PID: i32 = unsafe { libc::getpid() };
            static ref MY_PID_PSTART: u64 = tools::procfs::read_proc_pid_stat(*MY_PID).unwrap().starttime;
        }

        if (self.pid == *MY_PID) && (self.pstart == *MY_PID_PSTART) {
            if WORKER_TASK_LIST.lock().unwrap().contains_key(&self.task_id) {
                true
            } else {
                false
            }
        } else {
            match tools::procfs::check_process_running_pstart(self.pid, self.pstart) {
                Some(_) => true,
                _ => false,
            }
        }
    }
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

fn parse_worker_status_line(line: &str) -> Result<(String, UPID, Option<(i64, String)>), Error> {

    let data = line.splitn(3, ' ').collect::<Vec<&str>>();

    let len = data.len();

    match len {
        1 => Ok((data[0].to_owned(), data[0].parse::<UPID>()?, None)),
        3 => {
            let endtime = i64::from_str_radix(data[1], 16)?;
            Ok((data[0].to_owned(), data[0].parse::<UPID>()?, Some((endtime, data[2].to_owned()))))
        }
        _ => bail!("wrong number of components"),
    }
}

/// Create task log directory with correct permissions
pub fn create_task_log_dir() -> Result<(), Error> {

    try_block!({
        let (backup_uid, backup_gid) = tools::getpwnam_ugid("backup")?;
        let uid = Some(nix::unistd::Uid::from_raw(backup_uid));
        let gid = Some(nix::unistd::Gid::from_raw(backup_gid));

        tools::create_dir_chown(PROXMOX_BACKUP_LOG_DIR!(), None, uid, gid)?;
        tools::create_dir_chown(PROXMOX_BACKUP_TASK_DIR!(), None, uid, gid)?;

        Ok(())
    }).map_err(|err: Error| format_err!("unable to create task log dir - {}", err))?;

    Ok(())
}

fn upid_read_status(upid: &UPID) -> Result<String, Error> {
    let mut status = String::from("unknown");

    let path = upid.log_path();

    let file = File::open(path)?;
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line?;

        let mut iter = line.splitn(2, ": TASK ");
        if iter.next() == None { continue; }
        match iter.next() {
            None => continue,
            Some(rest) => {
                if rest == "OK" {
                    status = String::from(rest);
                } else if rest.starts_with("ERROR: ") {
                    status = String::from(rest);
                }
            }
        }
    }

    Ok(status)
}

/// Task details including parsed UPID
///
/// If there is no `state`, the task is still running.
#[derive(Debug)]
pub struct TaskListInfo {
    /// The parsed UPID
    pub upid: UPID,
    /// UPID string representation
    pub upid_str: String,
    /// Task `(endtime, status)` if already finished
    ///
    /// The `status` ise iether `unknown`, `OK`, or `ERROR: ...`
    pub state: Option<(i64, String)>, // endtime, status
}

// atomically read/update the task list, update status of finished tasks
// new_upid is added to the list when specified.
// Returns a sorted list of known tasks,
fn update_active_workers(new_upid: Option<&UPID>) -> Result<Vec<TaskListInfo>, Error> {

    let (backup_uid, backup_gid) = tools::getpwnam_ugid("backup")?;
    let uid = Some(nix::unistd::Uid::from_raw(backup_uid));
    let gid = Some(nix::unistd::Gid::from_raw(backup_gid));

    let lock = tools::open_file_locked(PROXMOX_BACKUP_TASK_LOCK_FN!(), std::time::Duration::new(10, 0))?;
    nix::unistd::chown(PROXMOX_BACKUP_TASK_LOCK_FN!(), uid, gid)?;

    let reader = match File::open(PROXMOX_BACKUP_ACTIVE_TASK_FN!()) {
        Ok(f) => Some(BufReader::new(f)),
        Err(err) => {
            if err.kind() ==  std::io::ErrorKind::NotFound {
                 None
            } else {
                bail!("unable to open active worker {:?} - {}", PROXMOX_BACKUP_ACTIVE_TASK_FN!(), err);
            }
        }
    };

    let mut active_list = vec![];
    let mut finish_list = vec![];

    if let Some(lines) = reader.map(|r| r.lines()) {

        for line in lines {
            let line = line?;
            match parse_worker_status_line(&line) {
                Err(err) => bail!("unable to parse active worker status '{}' - {}", line, err),
                Ok((upid_str, upid, state)) => {

                    let running = upid.is_active();

                    if running {
                        active_list.push(TaskListInfo { upid, upid_str, state: None });
                    } else {
                        match state {
                            None => {
                                println!("Detected stoped UPID {}", upid_str);
                                let status = upid_read_status(&upid).unwrap_or(String::from("unknown"));
                                finish_list.push(TaskListInfo {
                                    upid, upid_str, state: Some((Local::now().timestamp(), status))
                                });
                            }
                            Some((endtime, status)) => {
                                finish_list.push(TaskListInfo {
                                    upid, upid_str, state: Some((endtime, status))
                                })
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some(upid) = new_upid {
        active_list.push(TaskListInfo { upid: upid.clone(), upid_str: upid.to_string(), state: None });
    }

    // assemble list without duplicates
    // we include all active tasks,
    // and fill up to 1000 entries with finished tasks

    let max = 1000;

    let mut task_hash = HashMap::new();

    for info in active_list {
        task_hash.insert(info.upid_str.clone(), info);
    }

    for info in finish_list {
        if task_hash.len() > max { break; }
        if !task_hash.contains_key(&info.upid_str) {
            task_hash.insert(info.upid_str.clone(), info);
        }
    }

    let mut task_list: Vec<TaskListInfo> = vec![];
    for (_, info) in task_hash { task_list.push(info); }

    task_list.sort_unstable_by(|a, b| {
        match (&a.state, &b.state) {
            (Some(s1), Some(s2)) => s1.0.cmp(&s2.0),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => a.upid.starttime.cmp(&b.upid.starttime),
        }
    });

    let mut raw = String::new();
    for info in &task_list {
        if let Some((endtime, status)) = &info.state {
            raw.push_str(&format!("{} {:08X} {}\n", info.upid_str, endtime, status));
        } else {
            raw.push_str(&info.upid_str);
            raw.push('\n');
        }
    }

    tools::file_set_contents_full(PROXMOX_BACKUP_ACTIVE_TASK_FN!(), raw.as_bytes(), None, uid, gid)?;

    drop(lock);

    Ok(task_list)
}

/// Returns a sorted list of known tasks
///
/// The list is sorted by `(starttime, endtime)` in ascending order
pub fn read_task_list() -> Result<Vec<TaskListInfo>, Error> {
    update_active_workers(None)
}

/// Launch long running worker tasks.
///
/// A worker task can either be a whole thread, or a simply tokio
/// task/future. Each task can `log()` messages, which are stored
/// persistently to files. Task should poll the `abort_requested`
/// flag, and stop execution when requested.
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

        let (backup_uid, backup_gid) = tools::getpwnam_ugid("backup")?;
        let uid = Some(nix::unistd::Uid::from_raw(backup_uid));
        let gid = Some(nix::unistd::Gid::from_raw(backup_gid));

        tools::create_dir_chown(&path, None, uid, gid)?;

        path.push(upid.to_string());

        println!("FILE: {:?}", path);

        let logger = FileLogger::new(&path, to_stdout)?;
        nix::unistd::chown(&path, uid, gid)?;

        update_active_workers(Some(&upid))?;

        let worker = Arc::new(Self {
            upid: upid,
            abort_requested: AtomicBool::new(false),
            data: Mutex::new(WorkerTaskData {
                logger,
                progress: 0.0,
            }),
        });

        let mut hash = WORKER_TASK_LIST.lock().unwrap();

        hash.insert(task_id, worker.clone());
        super::set_worker_count(hash.len());

        Ok(worker)
    }

    /// Spawn a new tokio task/future.
    pub fn spawn<F, T>(
        worker_type: &str,
        worker_id: Option<String>,
        username: &str,
        to_stdout: bool,
        f: F,
    ) -> Result<String, Error>
        where F: Send + 'static + FnOnce(Arc<WorkerTask>) -> T,
              T: Send + 'static + Future<Item=(), Error=Error>,
    {
        let worker = WorkerTask::new(worker_type, worker_id, username, to_stdout)?;
        let task_id = worker.upid.task_id;
        let upid_str = worker.upid.to_string();

        tokio::spawn(f(worker.clone()).then(move |result| {
            WORKER_TASK_LIST.lock().unwrap().remove(&task_id);
            worker.log_result(result);
            let _ = update_active_workers(None);
            super::set_worker_count(WORKER_TASK_LIST.lock().unwrap().len());
            Ok(())
        }));

        Ok(upid_str)
    }

    /// Create a new worker thread.
    pub fn new_thread<F>(
        worker_type: &str,
        worker_id: Option<String>,
        username: &str,
        to_stdout: bool,
        f: F,
    ) -> Result<String, Error>
        where F: Send + 'static + FnOnce(Arc<WorkerTask>) -> Result<(), Error>
    {
        println!("register worker thread");

        let (p, c) = oneshot::channel::<()>();

        let worker = WorkerTask::new(worker_type, worker_id, username, to_stdout)?;
        let task_id = worker.upid.task_id;
        let upid_str = worker.upid.to_string();

        let _child = std::thread::spawn(move || {
            let result = f(worker.clone());
            WORKER_TASK_LIST.lock().unwrap().remove(&task_id);
            worker.log_result(result);
            let _ = update_active_workers(None);
            p.send(()).unwrap();
            super::set_worker_count(WORKER_TASK_LIST.lock().unwrap().len());
        });

        tokio::spawn(c.then(|_| Ok(())));

        Ok(upid_str)
    }

    fn log_result(&self, result: Result<(), Error>) {
        if let Err(err) = result {
            self.log(&format!("TASK ERROR: {}", err));
        } else {
            self.log("TASK OK");
        }
    }

    /// Log a message.
    pub fn log<S: AsRef<str>>(&self, msg: S) {
        let mut data = self.data.lock().unwrap();
        data.logger.log(msg);
    }

    /// Set progress indicator
    pub fn progress(&self, progress: f64) {
        if progress >= 0.0 && progress <= 1.0 {
            let mut data = self.data.lock().unwrap();
            data.progress = progress;
        } else {
           // fixme:  log!("task '{}': ignoring strange value for progress '{}'", self.upid, progress);
        }
    }

    /// Request abort
    pub fn request_abort(self) {
        self.abort_requested.store(true, Ordering::SeqCst);
    }

    /// Test if abort was requested.
    pub fn abort_requested(&self) -> bool {
        self.abort_requested.load(Ordering::SeqCst)
    }

    /// Fail if abort was requested.
    pub fn fail_on_abort(&self) -> Result<(), Error> {
        if self.abort_requested() {
            bail!("task '{}': abort requested - aborting task", self.upid);
        }
        Ok(())
    }
}
