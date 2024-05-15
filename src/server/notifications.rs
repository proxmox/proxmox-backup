use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Error;
use const_format::concatcp;
use nix::unistd::Uid;
use serde_json::json;

use proxmox_notify::context::pbs::PBS_CONTEXT;
use proxmox_schema::ApiType;
use proxmox_sys::fs::{create_path, CreateOptions};

use crate::tape::TapeNotificationMode;
use pbs_api_types::{
    APTUpdateInfo, DataStoreConfig, DatastoreNotify, GarbageCollectionStatus, NotificationMode,
    Notify, SyncJobConfig, TapeBackupJobSetup, User, Userid, VerificationJobConfig,
};
use proxmox_notify::endpoints::sendmail::{SendmailConfig, SendmailEndpoint};
use proxmox_notify::{Endpoint, Notification, Severity};

const SPOOL_DIR: &str = concatcp!(pbs_buildcfg::PROXMOX_BACKUP_STATE_DIR, "/notifications");

/// Initialize the notification system by setting context in proxmox_notify
pub fn init() -> Result<(), Error> {
    proxmox_notify::context::set_context(&PBS_CONTEXT);
    Ok(())
}

/// Create the directory which will be used to temporarily store notifications
/// which were sent from an unprivileged process.
pub fn create_spool_dir() -> Result<(), Error> {
    let backup_user = pbs_config::backup_user()?;
    let opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);

    create_path(SPOOL_DIR, None, Some(opts))?;
    Ok(())
}

async fn send_queued_notifications() -> Result<(), Error> {
    let mut read_dir = tokio::fs::read_dir(SPOOL_DIR).await?;

    let mut notifications = Vec::new();

    while let Some(entry) = read_dir.next_entry().await? {
        let path = entry.path();

        if let Some(ext) = path.extension() {
            if ext == "json" {
                let p = path.clone();

                let bytes = tokio::fs::read(p).await?;
                let notification: Notification = serde_json::from_slice(&bytes)?;
                notifications.push(notification);

                // Currently, there is no retry-mechanism in case of failure...
                // For retries, we'd have to keep track of which targets succeeded/failed
                // to send, so we do not retry notifying a target which succeeded before.
                tokio::fs::remove_file(path).await?;
            }
        }
    }

    // Make sure that we send the oldest notification first
    notifications.sort_unstable_by_key(|n| n.timestamp());

    let res = tokio::task::spawn_blocking(move || {
        let config = pbs_config::notifications::config()?;
        for notification in notifications {
            if let Err(err) = proxmox_notify::api::common::send(&config, &notification) {
                log::error!("failed to send notification: {err}");
            }
        }

        Ok::<(), Error>(())
    })
    .await?;

    if let Err(e) = res {
        log::error!("could not read notification config: {e}");
    }

    Ok::<(), Error>(())
}

/// Worker task to periodically send any queued notifications.
pub async fn notification_worker() {
    loop {
        let delay_target = Instant::now() + Duration::from_secs(5);

        if let Err(err) = send_queued_notifications().await {
            log::error!("notification worker task error: {err}");
        }

        tokio::time::sleep_until(tokio::time::Instant::from_std(delay_target)).await;
    }
}

fn send_notification(notification: Notification) -> Result<(), Error> {
    if nix::unistd::ROOT == Uid::current() {
        let config = pbs_config::notifications::config()?;
        proxmox_notify::api::common::send(&config, &notification)?;
    } else {
        let ser = serde_json::to_vec(&notification)?;
        let path = Path::new(SPOOL_DIR).join(format!("{id}.json", id = notification.id()));

        let backup_user = pbs_config::backup_user()?;
        let opts = CreateOptions::new()
            .owner(backup_user.uid)
            .group(backup_user.gid);
        proxmox_sys::fs::replace_file(path, &ser, opts, true)?;
        log::info!("queued notification (id={id})", id = notification.id())
    }

    Ok(())
}

fn send_sendmail_legacy_notification(notification: Notification, email: &str) -> Result<(), Error> {
    let endpoint = SendmailEndpoint {
        config: SendmailConfig {
            mailto: vec![email.into()],
            ..Default::default()
        },
    };

    endpoint.send(&notification)?;

    Ok(())
}

/// Summary of a successful Tape Job
#[derive(Default)]
pub struct TapeBackupJobSummary {
    /// The list of snaphots backed up
    pub snapshot_list: Vec<String>,
    /// The total time of the backup job
    pub duration: std::time::Duration,
    /// The labels of the used tapes of the backup job
    pub used_tapes: Option<Vec<String>>,
}

pub fn send_gc_status(
    datastore: &str,
    status: &GarbageCollectionStatus,
    result: &Result<(), Error>,
) -> Result<(), Error> {
    let (fqdn, port) = get_server_url();
    let mut data = json!({
        "datastore": datastore,
        "fqdn": fqdn,
        "port": port,
    });

    let (severity, template) = match result {
        Ok(()) => {
            let deduplication_factor = if status.disk_bytes > 0 {
                (status.index_data_bytes as f64) / (status.disk_bytes as f64)
            } else {
                1.0
            };

            data["status"] = json!(status);
            data["deduplication-factor"] = format!("{:.2}", deduplication_factor).into();

            (Severity::Info, "gc-ok")
        }
        Err(err) => {
            data["error"] = err.to_string().into();
            (Severity::Error, "gc-err")
        }
    };
    let metadata = HashMap::from([
        ("datastore".into(), datastore.into()),
        ("hostname".into(), proxmox_sys::nodename().into()),
        ("type".into(), "gc".into()),
    ]);

    let notification = Notification::from_template(severity, template, data, metadata);

    let (email, notify, mode) = lookup_datastore_notify_settings(datastore);
    match mode {
        NotificationMode::LegacySendmail => {
            let notify = notify.gc.unwrap_or(Notify::Always);

            if notify == Notify::Never || (result.is_ok() && notify == Notify::Error) {
                return Ok(());
            }

            if let Some(email) = email {
                send_sendmail_legacy_notification(notification, &email)?;
            }
        }
        NotificationMode::NotificationSystem => {
            send_notification(notification)?;
        }
    }

    Ok(())
}

pub fn send_verify_status(
    job: VerificationJobConfig,
    result: &Result<Vec<String>, Error>,
) -> Result<(), Error> {
    let (fqdn, port) = get_server_url();
    let mut data = json!({
        "job": job,
        "fqdn": fqdn,
        "port": port,
    });

    let (template, severity) = match result {
        Ok(errors) if errors.is_empty() => ("verify-ok", Severity::Info),
        Ok(errors) => {
            data["errors"] = json!(errors);
            ("verify-err", Severity::Error)
        }
        Err(_) => {
            // aborted job - do not send any notification
            return Ok(());
        }
    };

    let metadata = HashMap::from([
        ("job-id".into(), job.id.clone()),
        ("datastore".into(), job.store.clone()),
        ("hostname".into(), proxmox_sys::nodename().into()),
        ("type".into(), "verify".into()),
    ]);

    let notification = Notification::from_template(severity, template, data, metadata);

    let (email, notify, mode) = lookup_datastore_notify_settings(&job.store);
    match mode {
        NotificationMode::LegacySendmail => {
            let notify = notify.verify.unwrap_or(Notify::Always);

            if notify == Notify::Never || (result.is_ok() && notify == Notify::Error) {
                return Ok(());
            }

            if let Some(email) = email {
                send_sendmail_legacy_notification(notification, &email)?;
            }
        }
        NotificationMode::NotificationSystem => {
            send_notification(notification)?;
        }
    }

    Ok(())
}

pub fn send_prune_status(
    store: &str,
    jobname: &str,
    result: &Result<(), Error>,
) -> Result<(), Error> {
    let (fqdn, port) = get_server_url();
    let mut data = json!({
        "jobname": jobname,
        "store": store,
        "fqdn": fqdn,
        "port": port,
    });

    let (template, severity) = match result {
        Ok(()) => ("prune-ok", Severity::Info),
        Err(err) => {
            data["error"] = err.to_string().into();
            ("prune-err", Severity::Error)
        }
    };

    let metadata = HashMap::from([
        ("job-id".into(), jobname.to_string()),
        ("datastore".into(), store.into()),
        ("hostname".into(), proxmox_sys::nodename().into()),
        ("type".into(), "prune".into()),
    ]);

    let notification = Notification::from_template(severity, template, data, metadata);

    let (email, notify, mode) = lookup_datastore_notify_settings(store);
    match mode {
        NotificationMode::LegacySendmail => {
            let notify = notify.prune.unwrap_or(Notify::Error);

            if notify == Notify::Never || (result.is_ok() && notify == Notify::Error) {
                return Ok(());
            }

            if let Some(email) = email {
                send_sendmail_legacy_notification(notification, &email)?;
            }
        }
        NotificationMode::NotificationSystem => {
            send_notification(notification)?;
        }
    }

    Ok(())
}

pub fn send_sync_status(job: &SyncJobConfig, result: &Result<(), Error>) -> Result<(), Error> {
    let (fqdn, port) = get_server_url();
    let mut data = json!({
        "job": job,
        "fqdn": fqdn,
        "port": port,
    });

    let (template, severity) = match result {
        Ok(()) => ("sync-ok", Severity::Info),
        Err(err) => {
            data["error"] = err.to_string().into();
            ("sync-err", Severity::Error)
        }
    };

    let metadata = HashMap::from([
        ("job-id".into(), job.id.clone()),
        ("datastore".into(), job.store.clone()),
        ("hostname".into(), proxmox_sys::nodename().into()),
        ("type".into(), "sync".into()),
    ]);

    let notification = Notification::from_template(severity, template, data, metadata);

    let (email, notify, mode) = lookup_datastore_notify_settings(&job.store);
    match mode {
        NotificationMode::LegacySendmail => {
            let notify = notify.sync.unwrap_or(Notify::Always);

            if notify == Notify::Never || (result.is_ok() && notify == Notify::Error) {
                return Ok(());
            }

            if let Some(email) = email {
                send_sendmail_legacy_notification(notification, &email)?;
            }
        }
        NotificationMode::NotificationSystem => {
            send_notification(notification)?;
        }
    }

    Ok(())
}

pub fn send_tape_backup_status(
    id: Option<&str>,
    job: &TapeBackupJobSetup,
    result: &Result<(), Error>,
    summary: TapeBackupJobSummary,
) -> Result<(), Error> {
    let (fqdn, port) = get_server_url();
    let duration: proxmox_time::TimeSpan = summary.duration.into();
    let mut data = json!({
        "job": job,
        "fqdn": fqdn,
        "port": port,
        "id": id,
        "snapshot-list": summary.snapshot_list,
        "used-tapes": summary.used_tapes,
        "job-duration": duration.to_string(),
    });

    let (template, severity) = match result {
        Ok(()) => ("tape-backup-ok", Severity::Info),
        Err(err) => {
            data["error"] = err.to_string().into();
            ("tape-backup-err", Severity::Error)
        }
    };

    let mut metadata = HashMap::from([
        ("datastore".into(), job.store.clone()),
        ("media-pool".into(), job.pool.clone()),
        ("hostname".into(), proxmox_sys::nodename().into()),
        ("type".into(), "tape-backup".into()),
    ]);

    if let Some(id) = id {
        metadata.insert("job-id".into(), id.into());
    }

    let notification = Notification::from_template(severity, template, data, metadata);

    let mode = TapeNotificationMode::from(job);

    match &mode {
        TapeNotificationMode::LegacySendmail { notify_user } => {
            let email = lookup_user_email(notify_user);

            if let Some(email) = email {
                send_sendmail_legacy_notification(notification, &email)?;
            }
        }
        TapeNotificationMode::NotificationSystem => {
            send_notification(notification)?;
        }
    }

    Ok(())
}

/// Send email to a person to request a manual media change
pub fn send_load_media_notification(
    mode: &TapeNotificationMode,
    changer: bool,
    device: &str,
    label_text: &str,
    reason: Option<String>,
) -> Result<(), Error> {
    let device_type = if changer { "changer" } else { "drive" };

    let data = json!({
        "device-type": device_type,
        "device": device,
        "label-text": label_text,
        "reason": reason,
        "is-changer": changer,
    });

    let metadata = HashMap::from([
        ("hostname".into(), proxmox_sys::nodename().into()),
        ("type".into(), "tape-load".into()),
    ]);
    let notification = Notification::from_template(Severity::Notice, "tape-load", data, metadata);

    match mode {
        TapeNotificationMode::LegacySendmail { notify_user } => {
            let email = lookup_user_email(notify_user);

            if let Some(email) = email {
                send_sendmail_legacy_notification(notification, &email)?;
            }
        }
        TapeNotificationMode::NotificationSystem => {
            send_notification(notification)?;
        }
    }

    Ok(())
}

fn get_server_url() -> (String, usize) {
    // user will surely request that they can change this

    let nodename = proxmox_sys::nodename();
    let mut fqdn = nodename.to_owned();

    if let Ok(resolv_conf) = crate::api2::node::dns::read_etc_resolv_conf() {
        if let Some(search) = resolv_conf["search"].as_str() {
            fqdn.push('.');
            fqdn.push_str(search);
        }
    }

    let port = 8007;

    (fqdn, port)
}

pub fn send_updates_available(updates: &[&APTUpdateInfo]) -> Result<(), Error> {
    let (fqdn, port) = get_server_url();
    let hostname = proxmox_sys::nodename().to_string();

    let data = json!({
        "fqdn": fqdn,
        "hostname": &hostname,
        "port": port,
        "updates": updates,
    });

    let metadata = HashMap::from([
        ("hostname".into(), hostname),
        ("type".into(), "package-updates".into()),
    ]);

    let notification =
        Notification::from_template(Severity::Info, "package-updates", data, metadata);

    send_notification(notification)?;
    Ok(())
}

/// send email on certificate renewal failure.
pub fn send_certificate_renewal_mail(result: &Result<(), Error>) -> Result<(), Error> {
    let error: String = match result {
        Err(e) => e.to_string(),
        _ => return Ok(()),
    };

    let (fqdn, port) = get_server_url();

    let data = json!({
        "fqdn": fqdn,
        "port": port,
        "error": error,
    });

    let metadata = HashMap::from([
        ("hostname".into(), proxmox_sys::nodename().into()),
        ("type".into(), "acme".into()),
    ]);

    let notification = Notification::from_template(Severity::Info, "acme-err", data, metadata);

    send_notification(notification)?;
    Ok(())
}

/// Lookup users email address
pub fn lookup_user_email(userid: &Userid) -> Option<String> {
    if let Ok(user_config) = pbs_config::user::cached_config() {
        if let Ok(user) = user_config.lookup::<User>("user", userid.as_str()) {
            return user.email;
        }
    }

    None
}

/// Lookup Datastore notify settings
pub fn lookup_datastore_notify_settings(
    store: &str,
) -> (Option<String>, DatastoreNotify, NotificationMode) {
    let mut email = None;

    let notify = DatastoreNotify {
        gc: None,
        verify: None,
        sync: None,
        prune: None,
    };

    let (config, _digest) = match pbs_config::datastore::config() {
        Ok(result) => result,
        Err(_) => return (email, notify, NotificationMode::default()),
    };

    let config: DataStoreConfig = match config.lookup("datastore", store) {
        Ok(result) => result,
        Err(_) => return (email, notify, NotificationMode::default()),
    };

    email = match config.notify_user {
        Some(ref userid) => lookup_user_email(userid),
        None => lookup_user_email(Userid::root_userid()),
    };

    let notification_mode = config.notification_mode.unwrap_or_default();
    let notify_str = config.notify.unwrap_or_default();

    if let Ok(value) = DatastoreNotify::API_SCHEMA.parse_property_string(&notify_str) {
        if let Ok(notify) = serde_json::from_value(value) {
            return (email, notify, notification_mode);
        }
    }

    (email, notify, notification_mode)
}
