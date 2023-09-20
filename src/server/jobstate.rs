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
//! # use proxmox_rest_server::TaskState;
//! # use proxmox_backup::server::jobstate::*;
//! # fn some_code() -> TaskState { TaskState::OK { endtime: 0 } }
//! # fn code() -> Result<(), Error> {
//! // locks the correct file under /var/lib
//! // or fails if someone else holds the lock
//! let mut job = match Job::new("jobtype", "jobname") {
//!     Ok(job) => job,
//!     Err(err) => bail!("could not lock jobstate"),
//! };
//!
//! // job holds the lock, we can start it
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
use std::path::{Path, PathBuf};

use anyhow::{bail, format_err, Error};
use serde::{Deserialize, Serialize};

use proxmox_sys::fs::{create_path, file_read_optional_string, replace_file, CreateOptions};

use proxmox_time::CalendarEvent;

use pbs_api_types::{JobScheduleStatus, UPID};
use pbs_buildcfg::PROXMOX_BACKUP_STATE_DIR_M;
use pbs_config::{open_backup_lockfile, BackupLockGuard};

use proxmox_rest_server::{upid_read_status, worker_is_active_local, TaskState};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Represents the State of a specific Job
pub enum JobState {
    /// A job was created at 'time', but never started/finished
    Created { time: i64 },
    /// The Job was last started in 'upid',
    Started { upid: String },
    /// The Job was last started in 'upid', which finished with 'state', and was last updated at 'updated'
    Finished {
        upid: String,
        state: TaskState,
        updated: Option<i64>,
    },
}

/// Represents a Job and holds the correct lock
pub struct Job {
    jobtype: String,
    jobname: String,
    /// The State of the job
    pub state: JobState,
    _lock: BackupLockGuard,
}

const JOB_STATE_BASEDIR: &str = concat!(PROXMOX_BACKUP_STATE_DIR_M!(), "/jobstates");

/// Create jobstate stat dir with correct permission
pub fn create_jobstate_dir() -> Result<(), Error> {
    let backup_user = pbs_config::backup_user()?;

    let opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);

    create_path(JOB_STATE_BASEDIR, Some(opts.clone()), Some(opts))
        .map_err(|err: Error| format_err!("unable to create job state dir - {err}"))?;

    Ok(())
}

fn get_path(jobtype: &str, jobname: &str) -> PathBuf {
    let mut path = PathBuf::from(JOB_STATE_BASEDIR);
    path.push(format!("{jobtype}-{jobname}.json"));
    path
}

fn get_lock<P>(path: P) -> Result<BackupLockGuard, Error>
where
    P: AsRef<Path>,
{
    let mut path = path.as_ref().to_path_buf();
    path.set_extension("lck");
    open_backup_lockfile(&path, None, true)
}

/// Removes the statefile of a job, this is useful if we delete a job
pub fn remove_state_file(jobtype: &str, jobname: &str) -> Result<(), Error> {
    let mut path = get_path(jobtype, jobname);
    let _lock = get_lock(&path)?;
    if let Err(err) = std::fs::remove_file(&path) {
        if err.kind() != std::io::ErrorKind::NotFound {
            bail!("cannot remove statefile for {jobtype} - {jobname}: {err}");
        }
    }
    path.set_extension("lck");
    if let Err(err) = std::fs::remove_file(&path) {
        if err.kind() != std::io::ErrorKind::NotFound {
            bail!("cannot remove lockfile for {jobtype} - {jobname}: {err}");
        }
    }
    Ok(())
}

/// Creates the statefile with the state 'Created'
/// overwrites if it exists already
pub fn create_state_file(jobtype: &str, jobname: &str) -> Result<(), Error> {
    let mut job = Job::new(jobtype, jobname)?;
    job.write_state()
}

/// Tries to update the state file with the current time
/// if the job is currently running, does nothing.
/// Intended for use when the schedule changes.
pub fn update_job_last_run_time(jobtype: &str, jobname: &str) -> Result<(), Error> {
    let mut job = match Job::new(jobtype, jobname) {
        Ok(job) => job,
        Err(_) => return Ok(()), // was locked (running), so do not update
    };
    let time = proxmox_time::epoch_i64();

    job.state = match JobState::load(jobtype, jobname)? {
        JobState::Created { .. } => JobState::Created { time },
        JobState::Started { .. } => return Ok(()), // currently running (without lock?)
        JobState::Finished {
            upid,
            state,
            updated: _,
        } => JobState::Finished {
            upid,
            state,
            updated: Some(time),
        },
    };
    job.write_state()
}

/// Returns the last run time of a job by reading the statefile
/// Note that this is not locked
pub fn last_run_time(jobtype: &str, jobname: &str) -> Result<i64, Error> {
    match JobState::load(jobtype, jobname)? {
        JobState::Created { time } => Ok(time),
        JobState::Finished {
            updated: Some(time),
            ..
        } => Ok(time),
        JobState::Started { upid }
        | JobState::Finished {
            upid,
            state: _,
            updated: None,
        } => {
            let upid: UPID = upid
                .parse()
                .map_err(|err| format_err!("could not parse upid from state: {err}"))?;
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
                    let parsed: UPID = upid
                        .parse()
                        .map_err(|err| format_err!("error parsing upid: {err}"))?;

                    if !worker_is_active_local(&parsed) {
                        let state = upid_read_status(&parsed).unwrap_or(TaskState::Unknown {
                            endtime: parsed.starttime,
                        });

                        Ok(JobState::Finished {
                            upid,
                            state,
                            updated: None,
                        })
                    } else {
                        Ok(JobState::Started { upid })
                    }
                }
                other => Ok(other),
            }
        } else {
            Ok(JobState::Created {
                time: proxmox_time::epoch_i64() - 30,
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

        let _lock = get_lock(path)?;

        Ok(Self {
            jobtype: jobtype.to_string(),
            jobname: jobname.to_string(),
            state: JobState::Created {
                time: proxmox_time::epoch_i64(),
            },
            _lock,
        })
    }

    /// Start the job and update the statefile accordingly
    /// Fails if the job was already started
    pub fn start(&mut self, upid: &str) -> Result<(), Error> {
        if let JobState::Started { .. } = self.state {
            bail!("cannot start job that is started!");
        }

        self.state = JobState::Started {
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
        }
        .to_string();

        self.state = JobState::Finished {
            upid,
            state,
            updated: None,
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

        let backup_user = pbs_config::backup_user()?;
        let mode = nix::sys::stat::Mode::from_bits_truncate(0o0644);
        // set the correct owner/group/permissions while saving file
        // owner(rw) = backup, group(r)= backup
        let options = CreateOptions::new()
            .perm(mode)
            .owner(backup_user.uid)
            .group(backup_user.gid);

        replace_file(path, serialized.as_bytes(), options, false)
    }
}

pub fn compute_schedule_status(
    job_state: &JobState,
    schedule: Option<&str>,
) -> Result<JobScheduleStatus, Error> {
    let (upid, endtime, state, last) = match job_state {
        JobState::Created { time } => (None, None, None, *time),
        JobState::Started { upid } => {
            let parsed_upid: UPID = upid.parse()?;
            (Some(upid), None, None, parsed_upid.starttime)
        }
        JobState::Finished {
            upid,
            state,
            updated,
        } => {
            let last = updated.unwrap_or_else(|| state.endtime());
            (
                Some(upid),
                Some(state.endtime()),
                Some(state.to_string()),
                last,
            )
        }
    };

    let mut status = JobScheduleStatus {
        last_run_upid: upid.map(String::from),
        last_run_state: state,
        last_run_endtime: endtime,
        ..Default::default()
    };

    if let Some(schedule) = schedule {
        if let Ok(event) = schedule.parse::<CalendarEvent>() {
            // ignore errors
            status.next_run = event.compute_next_event(last).unwrap_or(None);
        }
    }

    Ok(status)
}
