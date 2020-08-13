//! Generic JobState handling
//!
//! A 'Job' can have 3 states
//!  - Created, when a schedule was created but never executed
//!  - Started, when a job is running right now
//!  - Finished, when a job was running in the past
//!
//! and is identified by 2 values: jobtype and jobname (e.g. 'syncjob' and 'myfirstsyncjob')
//!
//! This module Provides 2 helper structs to handle those coniditons
//! 'Job' which handles locking and writing to a file
//! 'JobState' which is the actual state
//!
//! an example usage would be
//! ```no_run
//! # use anyhow::{bail, Error};
//! # use proxmox_backup::server::TaskState;
//! # use proxmox_backup::config::jobstate::*;
//! # fn some_code() -> TaskState { TaskState::OK { endtime: 0 } }
//! # fn code() -> Result<(), Error> {
//! // locks the correct file under /var/lib
//! // or fails if someone else holds the lock
//! let mut job = match Job::new("jobtype", "jobname") {
//!     Ok(job) => job,
//!     Err(err) => bail!("could not lock jobstate"),
//! };
//!
//! // job holds the lock
//! match job.load() {
//!     Ok(()) => {},
//!     Err(err) => bail!("could not load state {}", err),
//! }
//!
//! // now the job is loaded;
//! job.start("someupid")?;
//! // do something
//! let task_state = some_code();
//! job.finish(task_state)?;
//!
//! // release the lock
//! drop(job);
//! # Ok(())
//! # }
//!
//! ```
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Serialize, Deserialize};
use anyhow::{bail, Error, format_err};
use proxmox::tools::fs::{file_read_optional_string, replace_file, create_path, CreateOptions, open_file_locked};

use crate::tools::epoch_now_u64;
use crate::server::{TaskState, UPID, worker_is_active_local, upid_read_status};

#[serde(rename_all="kebab-case")]
#[derive(Serialize,Deserialize)]
/// Represents the State of a specific Job
pub enum JobState {
    /// A job was created at 'time', but never started/finished
    Created { time: i64 },
    /// The Job was last started in 'upid',
    Started { upid: String },
    /// The Job was last started in 'upid', which finished with 'state'
    Finished { upid: String, state: TaskState }
}

/// Represents a Job and holds the correct lock
pub struct Job {
    jobtype: String,
    jobname: String,
    /// The State of the job
    pub state: JobState,
    _lock: File,
}

const JOB_STATE_BASEDIR: &str = "/var/lib/proxmox-backup/jobstates";

/// Create jobstate stat dir with correct permission
pub fn create_jobstate_dir() -> Result<(), Error> {
    let backup_user = crate::backup::backup_user()?;
    let opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);

    create_path(JOB_STATE_BASEDIR, None, Some(opts))
        .map_err(|err: Error| format_err!("unable to create rrdb stat dir - {}", err))?;

    Ok(())
}

fn get_path(jobtype: &str, jobname: &str) -> PathBuf {
    let mut path = PathBuf::from(JOB_STATE_BASEDIR);
    path.push(format!("{}-{}.json", jobtype, jobname));
    path
}

fn get_lock<P>(path: P) -> Result<File, Error>
where
    P: AsRef<Path>
{
    let mut path = path.as_ref().to_path_buf();
    path.set_extension("lck");
    open_file_locked(path, Duration::new(10, 0))
}

/// Removes the statefile of a job, this is useful if we delete a job
pub fn remove_state_file(jobtype: &str, jobname: &str) -> Result<(), Error> {
    let path = get_path(jobtype, jobname);
    let _lock = get_lock(&path)?;
    std::fs::remove_file(&path).map_err(|err|
        format_err!("cannot remove statefile for {} - {}: {}", jobtype, jobname, err)
    )
}

/// Returns the last run time of a job by reading the statefile
/// Note that this is not locked
pub fn last_run_time(jobtype: &str, jobname: &str) -> Result<i64, Error> {
    match JobState::load(jobtype, jobname)? {
        JobState::Created { time } => Ok(time),
        JobState::Started { upid } | JobState::Finished { upid, .. } => {
            let upid: UPID = upid.parse().map_err(|err|
                format_err!("could not parse upid from state: {}", err)
            )?;
            Ok(upid.starttime)
        }
    }
}

impl JobState {
    /// Loads and deserializes the jobstate from type and name.
    /// When the loaded state indicates a started UPID,
    /// we go and check if it has already stopped, and
    /// returning the correct state.
    ///
    /// This does not update the state in the file.
    pub fn load(jobtype: &str, jobname: &str) -> Result<Self, Error> {
        if let Some(state) = file_read_optional_string(get_path(jobtype, jobname))? {
            match serde_json::from_str(&state)? {
                JobState::Started { upid } => {
                    let parsed: UPID = upid.parse()
                        .map_err(|err| format_err!("error parsing upid: {}", err))?;

                    if !worker_is_active_local(&parsed) {
                        let state = upid_read_status(&parsed)
                            .map_err(|err| format_err!("error reading upid log status: {}", err))?;

                        Ok(JobState::Finished {
                            upid,
                            state
                        })
                    } else {
                        Ok(JobState::Started { upid })
                    }
                }
                other => Ok(other),
            }
        } else {
            Ok(JobState::Created {
                time: epoch_now_u64()? as i64
            })
        }
    }
}

impl Job {
    /// Creates a new instance of a job with the correct lock held
    /// (will be hold until the job is dropped again).
    ///
    /// This does not load the state from the file, to do that,
    /// 'load' must be called
    pub fn new(jobtype: &str, jobname: &str) -> Result<Self, Error> {
        let path = get_path(jobtype, jobname);

        let _lock = get_lock(&path)?;

        Ok(Self{
            jobtype: jobtype.to_string(),
            jobname: jobname.to_string(),
            state: JobState::Created {
                time: epoch_now_u64()? as i64
            },
            _lock,
        })
    }

    /// Loads the state from the statefile if it exists.
    /// If not, it gets created. Updates 'Started' State to 'Finished'
    /// if we detect the UPID already stopped
    pub fn load(&mut self) -> Result<(), Error> {
        self.state = JobState::load(&self.jobtype, &self.jobname)?;

        if let Err(err) = self.write_state() {
            bail!("could not write statefile: {}", err);
        }

        Ok(())
    }

    /// Start the job and update the statefile accordingly
    /// Fails if the job was already started
    pub fn start(&mut self, upid: &str) -> Result<(), Error> {
        match self.state {
            JobState::Started { .. } => {
                bail!("cannot start job that is started!");
            }
            _ => {}
        }

        self.state = JobState::Started{
            upid: upid.to_string(),
        };

        self.write_state()
    }

    /// Finish the job and update the statefile accordingly with the given taskstate
    /// Fails if the job was not yet started
    pub fn finish(&mut self, state: TaskState) -> Result<(), Error> {
        let upid = match &self.state {
            JobState::Created { .. } => bail!("cannot finish when not started"),
            JobState::Started { upid } => upid,
            JobState::Finished { upid, .. } => upid,
        }.to_string();

        self.state = JobState::Finished {
            upid,
            state,
        };

        self.write_state()
    }

    pub fn jobtype(&self) -> &str {
        &self.jobtype
    }

    pub fn jobname(&self) -> &str {
        &self.jobname
    }

    fn write_state(&mut self) -> Result<(), Error> {
        let serialized = serde_json::to_string(&self.state)?;
        let path = get_path(&self.jobtype, &self.jobname);

        let backup_user = crate::backup::backup_user()?;
        let mode = nix::sys::stat::Mode::from_bits_truncate(0o0644);
        // set the correct owner/group/permissions while saving file
        // owner(rw) = backup, group(r)= backup
        let options = CreateOptions::new()
            .perm(mode)
            .owner(backup_user.uid)
            .group(backup_user.gid);

        replace_file(
            path,
            serialized.as_bytes(),
            options,
        )
    }
}
