use failure::*;
use lazy_static::lazy_static;
use chrono::Local;

use tokio::sync::oneshot;
use futures::*;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::io::{BufRead, BufReader};
use std::fs::File;
use std::panic::UnwindSafe;

use serde_json::{json, Value};

use super::UPID;

use crate::tools::{self, FileLogger};

macro_rules! PROXMOX_BACKUP_VAR_RUN_DIR_M { () => ("/var/run/proxmox-backup") }
macro_rules! PROXMOX_BACKUP_LOG_DIR_M { () => ("/var/log/proxmox-backup") }
macro_rules! PROXMOX_BACKUP_TASK_DIR_M { () => (concat!( PROXMOX_BACKUP_LOG_DIR_M!(), "/tasks")) }

pub const PROXMOX_BACKUP_VAR_RUN_DIR: &str = PROXMOX_BACKUP_VAR_RUN_DIR_M!();
pub const PROXMOX_BACKUP_LOG_DIR: &str = PROXMOX_BACKUP_LOG_DIR_M!();
pub const PROXMOX_BACKUP_TASK_DIR: &str = PROXMOX_BACKUP_TASK_DIR_M!();
pub const PROXMOX_BACKUP_TASK_LOCK_FN: &str = concat!(PROXMOX_BACKUP_TASK_DIR_M!(), "/.active.lock");
pub const PROXMOX_BACKUP_ACTIVE_TASK_FN: &str = concat!(PROXMOX_BACKUP_TASK_DIR_M!(), "/active");

lazy_static! {
    static ref WORKER_TASK_LIST: Mutex<HashMap<usize, Arc<WorkerTask>>> = Mutex::new(HashMap::new());

    static ref MY_PID: i32 = unsafe { libc::getpid() };
    static ref MY_PID_PSTART: u64 = tools::procfs::read_proc_pid_stat(*MY_PID).unwrap().starttime;
}

/// Test if the task is still running
pub fn worker_is_active(upid: &UPID) -> bool {

    if (upid.pid == *MY_PID) && (upid.pstart == *MY_PID_PSTART) {
        if WORKER_TASK_LIST.lock().unwrap().contains_key(&upid.task_id) {
            true
        } else {
            false
        }
    } else {
        match tools::procfs::check_process_running_pstart(upid.pid, upid.pstart) {
            Some(_) => true,
            _ => false,
        }
    }
}

pub fn create_task_control_socket() -> Result<(), Error> {

    let socketname = format!(
        "\0{}/proxmox-task-control-{}.sock", PROXMOX_BACKUP_VAR_RUN_DIR, *MY_PID);

    let control_future = super::create_control_socket(socketname, |param| {
        let param = param.as_object()
            .ok_or(format_err!("unable to parse parameters (expected json object)"))?;
        if param.keys().count() != 2 { bail!("wrong number of parameters"); }

        let command = param.get("command")
            .ok_or(format_err!("unable to parse parameters (missing command)"))?;

        // this is the only command for now
        if command != "abort-task" { bail!("got unknown command '{}'", command); }

        let upid_str = param["upid"].as_str()
            .ok_or(format_err!("unable to parse parameters (missing upid)"))?;

        let upid = upid_str.parse::<UPID>()?;

        if !((upid.pid == *MY_PID) && (upid.pstart == *MY_PID_PSTART)) {
            bail!("upid does not belong to this process");
        }

        let hash = WORKER_TASK_LIST.lock().unwrap();
        if let Some(ref worker) = hash.get(&upid.task_id) {
            worker.request_abort();
        } else {
            // assume task is already stopped
        }
        Ok(Value::Null)
    })?;

    tokio::spawn(control_future);

    Ok(())
}

pub fn abort_worker_async(upid: UPID) {
    let task = abort_worker(upid);

    tokio::spawn(task.then(|res| {
        if let Err(err) = res {
            eprintln!("abort worker failed - {}", err);
        }
        Ok(())
    }));
}

pub fn abort_worker(upid: UPID) -> impl Future<Item=(), Error=Error> {

    let target_pid = upid.pid;

    let socketname = format!(
        "\0{}/proxmox-task-control-{}.sock", PROXMOX_BACKUP_VAR_RUN_DIR, target_pid);

    let cmd = json!({
        "command": "abort-task",
        "upid": upid.to_string(),
    });

    super::send_command(socketname, cmd).map(|_| {})
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
pub fn create_task_log_dirs() -> Result<(), Error> {

    try_block!({
        let (backup_uid, backup_gid) = tools::getpwnam_ugid("backup")?;
        let uid = Some(nix::unistd::Uid::from_raw(backup_uid));
        let gid = Some(nix::unistd::Gid::from_raw(backup_gid));

        tools::create_dir_chown(PROXMOX_BACKUP_LOG_DIR, None, uid, gid)?;
        tools::create_dir_chown(PROXMOX_BACKUP_TASK_DIR, None, uid, gid)?;
        tools::create_dir_chown(PROXMOX_BACKUP_VAR_RUN_DIR, None, uid, gid)?;
        Ok(())
    }).map_err(|err: Error| format_err!("unable to create task log dir - {}", err))?;

    Ok(())
}

/// Read exits status from task log file
pub fn upid_read_status(upid: &UPID) -> Result<String, Error> {
    let mut status = String::from("unknown");

    let path = upid.log_path();

    let mut file = File::open(path)?;

    /// speedup - only read tail
    use std::io::Seek;
    use std::io::SeekFrom;
    let _ = file.seek(SeekFrom::End(-8192)); // ignore errors

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

    let lock = tools::open_file_locked(PROXMOX_BACKUP_TASK_LOCK_FN, std::time::Duration::new(10, 0))?;
    nix::unistd::chown(PROXMOX_BACKUP_TASK_LOCK_FN, uid, gid)?;

    let reader = match File::open(PROXMOX_BACKUP_ACTIVE_TASK_FN) {
        Ok(f) => Some(BufReader::new(f)),
        Err(err) => {
            if err.kind() ==  std::io::ErrorKind::NotFound {
                 None
            } else {
                bail!("unable to open active worker {:?} - {}", PROXMOX_BACKUP_ACTIVE_TASK_FN, err);
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

                    let running = worker_is_active(&upid);

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

    task_list.sort_unstable_by(|b, a| { // lastest on top
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

    tools::file_set_contents_full(PROXMOX_BACKUP_ACTIVE_TASK_FN, raw.as_bytes(), None, uid, gid)?;

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

    pub fn new(worker_type: &str, worker_id: Option<String>, username: &str, to_stdout: bool) -> Result<Arc<Self>, Error> {
        println!("register worker");

        let upid = UPID::new(worker_type, worker_id, username)?;
        let task_id = upid.task_id;

        let mut path = std::path::PathBuf::from(PROXMOX_BACKUP_TASK_DIR);

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
        let upid_str = worker.upid.to_string();

        tokio::spawn(f(worker.clone()).then(move |result| {
            worker.log_result(result);
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
        where F: Send + UnwindSafe + 'static + FnOnce(Arc<WorkerTask>) -> Result<(), Error>
    {
        println!("register worker thread");

        let (p, c) = oneshot::channel::<()>();

        let worker = WorkerTask::new(worker_type, worker_id, username, to_stdout)?;
        let upid_str = worker.upid.to_string();

        let _child = std::thread::spawn(move || {
            let worker1 = worker.clone();
            let result = match std::panic::catch_unwind(move || f(worker1)) {
                Ok(r) => r,
                Err(panic) => {
                    match panic.downcast::<&str>() {
                        Ok(panic_msg) => {
                            Err(format_err!("worker panicked: {}", panic_msg))
                        }
                        Err(_) => {
                            Err(format_err!("worker panicked: unknown type."))
                        }
                    }
                }
            };

            worker.log_result(result);
            p.send(()).unwrap();
        });

        tokio::spawn(c.then(|_| Ok(())));

        Ok(upid_str)
    }

    /// Log task result, remove task from running list
    pub fn log_result(&self, result: Result<(), Error>) {

        if let Err(err) = result {
            self.log(&format!("TASK ERROR: {}", err));
        } else {
            self.log("TASK OK");
        }

        WORKER_TASK_LIST.lock().unwrap().remove(&self.upid.task_id);
        let _ = update_active_workers(None);
        super::set_worker_count(WORKER_TASK_LIST.lock().unwrap().len());
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
    pub fn request_abort(&self) {
        eprintln!("set abort flag for worker {}", self.upid);
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
