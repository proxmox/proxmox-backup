use anyhow::Error;
use serde_json::json;

use handlebars::{
    Context, Handlebars, Helper, HelperResult, Output, RenderContext, RenderError, TemplateError,
};

use proxmox_human_byte::HumanByte;
use proxmox_lang::try_block;
use proxmox_schema::ApiType;
use proxmox_sys::email::sendmail;

use pbs_api_types::{
    APTUpdateInfo, DataStoreConfig, DatastoreNotify, GarbageCollectionStatus, Notify,
    SyncJobConfig, TapeBackupJobSetup, User, Userid, VerificationJobConfig,
};

const GC_OK_TEMPLATE: &str = r###"

Datastore:            {{datastore}}
Task ID:              {{status.upid}}
Index file count:     {{status.index-file-count}}

Removed garbage:      {{human-bytes status.removed-bytes}}
Removed chunks:       {{status.removed-chunks}}
Removed bad chunks:   {{status.removed-bad}}

Leftover bad chunks:  {{status.still-bad}}
Pending removals:     {{human-bytes status.pending-bytes}} (in {{status.pending-chunks}} chunks)

Original Data usage:  {{human-bytes status.index-data-bytes}}
On-Disk usage:        {{human-bytes status.disk-bytes}} ({{relative-percentage status.disk-bytes status.index-data-bytes}})
On-Disk chunks:       {{status.disk-chunks}}

Deduplication Factor: {{deduplication-factor}}

Garbage collection successful.


Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#DataStore-{{datastore}}>

"###;

const GC_ERR_TEMPLATE: &str = r###"

Datastore: {{datastore}}

Garbage collection failed: {{error}}


Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#pbsServerAdministration:tasks>

"###;

const VERIFY_OK_TEMPLATE: &str = r###"

Job ID:    {{job.id}}
Datastore: {{job.store}}

Verification successful.


Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#DataStore-{{job.store}}>

"###;

const VERIFY_ERR_TEMPLATE: &str = r###"

Job ID:    {{job.id}}
Datastore: {{job.store}}

Verification failed on these snapshots/groups:

{{#each errors}}
  {{this~}}
{{/each}}


Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#pbsServerAdministration:tasks>

"###;

const SYNC_OK_TEMPLATE: &str = r###"

Job ID:             {{job.id}}
Datastore:          {{job.store}}
{{#if job.remote~}}
Remote:             {{job.remote}}
Remote Store:       {{job.remote-store}}
{{else~}}
Local Source Store: {{job.remote-store}}
{{/if}}
Synchronization successful.


Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#DataStore-{{job.store}}>

"###;

const SYNC_ERR_TEMPLATE: &str = r###"

Job ID:             {{job.id}}
Datastore:          {{job.store}}
{{#if job.remote~}}
Remote:             {{job.remote}}
Remote Store:       {{job.remote-store}}
{{else~}}
Local Source Store: {{job.remote-store}}
{{/if}}
Synchronization failed: {{error}}


Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#pbsServerAdministration:tasks>

"###;

const PRUNE_OK_TEMPLATE: &str = r###"

Job ID:       {{jobname}}
Datastore:    {{store}}

Pruning successful.


Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#DataStore-{{store}}>

"###;

const PRUNE_ERR_TEMPLATE: &str = r###"

Job ID:       {{jobname}}
Datastore:    {{store}}

Pruning failed: {{error}}


Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#pbsServerAdministration:tasks>

"###;

const PACKAGE_UPDATES_TEMPLATE: &str = r###"
Proxmox Backup Server has the following updates available:
{{#each updates }}
  {{Package}}: {{OldVersion}} -> {{Version~}}
{{/each }}

To upgrade visit the web interface:

<https://{{fqdn}}:{{port}}/#pbsServerAdministration:updates>

"###;

const TAPE_BACKUP_OK_TEMPLATE: &str = r###"

{{#if id ~}}
Job ID:     {{id}}
{{/if~}}
Datastore:  {{job.store}}
Tape Pool:  {{job.pool}}
Tape Drive: {{job.drive}}

{{#if snapshot-list ~}}
Snapshots included:

{{#each snapshot-list~}}
{{this}}
{{/each~}}
{{/if}}
Duration: {{duration}}
{{#if used-tapes }}
Used Tapes:
{{#each used-tapes~}}
{{this}}
{{/each~}}
{{/if}}
Tape Backup successful.


Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#DataStore-{{job.store}}>

"###;

const TAPE_BACKUP_ERR_TEMPLATE: &str = r###"

{{#if id ~}}
Job ID:     {{id}}
{{/if~}}
Datastore:  {{job.store}}
Tape Pool:  {{job.pool}}
Tape Drive: {{job.drive}}

{{#if snapshot-list ~}}
Snapshots included:

{{#each snapshot-list~}}
{{this}}
{{/each~}}
{{/if}}
{{#if used-tapes }}
Used Tapes:
{{#each used-tapes~}}
{{this}}
{{/each~}}
{{/if}}
Tape Backup failed: {{error}}


Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#pbsServerAdministration:tasks>

"###;

const ACME_CERTIFICATE_ERR_RENEWAL: &str = r###"

Proxmox Backup Server was not able to renew a TLS certificate.

Error: {{error}}

Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#pbsCertificateConfiguration>

"###;

lazy_static::lazy_static! {

    static ref HANDLEBARS: Handlebars<'static> = {
        let mut hb = Handlebars::new();
        let result: Result<(), TemplateError> = try_block!({

            hb.set_strict_mode(true);
            hb.register_escape_fn(handlebars::no_escape);

            hb.register_helper("human-bytes", Box::new(handlebars_humam_bytes_helper));
            hb.register_helper("relative-percentage", Box::new(handlebars_relative_percentage_helper));

            hb.register_template_string("gc_ok_template", GC_OK_TEMPLATE)?;
            hb.register_template_string("gc_err_template", GC_ERR_TEMPLATE)?;

            hb.register_template_string("verify_ok_template", VERIFY_OK_TEMPLATE)?;
            hb.register_template_string("verify_err_template", VERIFY_ERR_TEMPLATE)?;

            hb.register_template_string("sync_ok_template", SYNC_OK_TEMPLATE)?;
            hb.register_template_string("sync_err_template", SYNC_ERR_TEMPLATE)?;

            hb.register_template_string("prune_ok_template", PRUNE_OK_TEMPLATE)?;
            hb.register_template_string("prune_err_template", PRUNE_ERR_TEMPLATE)?;

            hb.register_template_string("tape_backup_ok_template", TAPE_BACKUP_OK_TEMPLATE)?;
            hb.register_template_string("tape_backup_err_template", TAPE_BACKUP_ERR_TEMPLATE)?;

            hb.register_template_string("package_update_template", PACKAGE_UPDATES_TEMPLATE)?;

            hb.register_template_string("certificate_renewal_err_template", ACME_CERTIFICATE_ERR_RENEWAL)?;

            Ok(())
        });

        if let Err(err) = result {
            eprintln!("error during template registration: {err}");
        }

        hb
    };
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

fn send_job_status_mail(email: &str, subject: &str, text: &str) -> Result<(), Error> {
    let (config, _) = crate::config::node::config()?;
    let from = config.email_from;

    // NOTE: some (web)mailers have big problems displaying text mails, so include html as well
    let escaped_text = handlebars::html_escape(text);
    let html = format!("<html><body><pre>\n{escaped_text}\n<pre>");

    let nodename = proxmox_sys::nodename();

    let author = format!("Proxmox Backup Server - {nodename}");

    sendmail(
        &[email],
        subject,
        Some(text),
        Some(&html),
        from.as_deref(),
        Some(&author),
    )?;

    Ok(())
}

pub fn send_gc_status(
    email: &str,
    notify: DatastoreNotify,
    datastore: &str,
    status: &GarbageCollectionStatus,
    result: &Result<(), Error>,
) -> Result<(), Error> {
    match notify.gc {
        None => { /* send notifications by default */ }
        Some(notify) => {
            if notify == Notify::Never || (result.is_ok() && notify == Notify::Error) {
                return Ok(());
            }
        }
    }

    let (fqdn, port) = get_server_url();
    let mut data = json!({
        "datastore": datastore,
        "fqdn": fqdn,
        "port": port,
    });

    let text = match result {
        Ok(()) => {
            let deduplication_factor = if status.disk_bytes > 0 {
                (status.index_data_bytes as f64) / (status.disk_bytes as f64)
            } else {
                1.0
            };

            data["status"] = json!(status);
            data["deduplication-factor"] = format!("{:.2}", deduplication_factor).into();

            HANDLEBARS.render("gc_ok_template", &data)?
        }
        Err(err) => {
            data["error"] = err.to_string().into();
            HANDLEBARS.render("gc_err_template", &data)?
        }
    };

    let subject = match result {
        Ok(()) => format!("Garbage Collect Datastore '{datastore}' successful"),
        Err(_) => format!("Garbage Collect Datastore '{datastore}' failed"),
    };

    send_job_status_mail(email, &subject, &text)?;

    Ok(())
}

pub fn send_verify_status(
    email: &str,
    notify: DatastoreNotify,
    job: VerificationJobConfig,
    result: &Result<Vec<String>, Error>,
) -> Result<(), Error> {
    let (fqdn, port) = get_server_url();
    let mut data = json!({
        "job": job,
        "fqdn": fqdn,
        "port": port,
    });

    let mut result_is_ok = false;

    let text = match result {
        Ok(errors) if errors.is_empty() => {
            result_is_ok = true;
            HANDLEBARS.render("verify_ok_template", &data)?
        }
        Ok(errors) => {
            data["errors"] = json!(errors);
            HANDLEBARS.render("verify_err_template", &data)?
        }
        Err(_) => {
            // aborted job - do not send any email
            return Ok(());
        }
    };

    match notify.verify {
        None => { /* send notifications by default */ }
        Some(notify) => {
            if notify == Notify::Never || (result_is_ok && notify == Notify::Error) {
                return Ok(());
            }
        }
    }

    let subject = match result {
        Ok(errors) if errors.is_empty() => format!("Verify Datastore '{}' successful", job.store),
        _ => format!("Verify Datastore '{}' failed", job.store),
    };

    send_job_status_mail(email, &subject, &text)?;

    Ok(())
}

pub fn send_prune_status(
    store: &str,
    jobname: &str,
    result: &Result<(), Error>,
) -> Result<(), Error> {
    let (email, notify) = match lookup_datastore_notify_settings(store) {
        (Some(email), notify) => (email, notify),
        (None, _) => return Ok(()),
    };

    let notify_prune = notify.prune.unwrap_or(Notify::Error);
    if notify_prune == Notify::Never || (result.is_ok() && notify_prune == Notify::Error) {
        return Ok(());
    }

    let (fqdn, port) = get_server_url();
    let mut data = json!({
        "jobname": jobname,
        "store": store,
        "fqdn": fqdn,
        "port": port,
    });

    let text = match result {
        Ok(()) => HANDLEBARS.render("prune_ok_template", &data)?,
        Err(err) => {
            data["error"] = err.to_string().into();
            HANDLEBARS.render("prune_err_template", &data)?
        }
    };

    let subject = match result {
        Ok(()) => format!("Pruning datastore '{store}' successful"),
        Err(_) => format!("Pruning datastore '{store}' failed"),
    };

    send_job_status_mail(&email, &subject, &text)?;

    Ok(())
}

pub fn send_sync_status(
    email: &str,
    notify: DatastoreNotify,
    job: &SyncJobConfig,
    result: &Result<(), Error>,
) -> Result<(), Error> {
    match notify.sync {
        None => { /* send notifications by default */ }
        Some(notify) => {
            if notify == Notify::Never || (result.is_ok() && notify == Notify::Error) {
                return Ok(());
            }
        }
    }

    let (fqdn, port) = get_server_url();
    let mut data = json!({
        "job": job,
        "fqdn": fqdn,
        "port": port,
    });

    let text = match result {
        Ok(()) => HANDLEBARS.render("sync_ok_template", &data)?,
        Err(err) => {
            data["error"] = err.to_string().into();
            HANDLEBARS.render("sync_err_template", &data)?
        }
    };

    let tmp_src_string;
    let source_str = if let Some(remote) = &job.remote {
        tmp_src_string = format!("Sync remote '{}'", remote);
        &tmp_src_string
    } else {
        "Sync local"
    };

    let subject = match result {
        Ok(()) => format!("{} datastore '{}' successful", source_str, job.remote_store,),
        Err(_) => format!("{} datastore '{}' failed", source_str, job.remote_store,),
    };

    send_job_status_mail(email, &subject, &text)?;

    Ok(())
}

pub fn send_tape_backup_status(
    email: &str,
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
        "duration": duration.to_string(),
    });

    let text = match result {
        Ok(()) => HANDLEBARS.render("tape_backup_ok_template", &data)?,
        Err(err) => {
            data["error"] = err.to_string().into();
            HANDLEBARS.render("tape_backup_err_template", &data)?
        }
    };

    let subject = match (result, id) {
        (Ok(()), Some(id)) => format!("Tape Backup '{id}' datastore '{}' successful", job.store,),
        (Ok(()), None) => format!("Tape Backup datastore '{}' successful", job.store,),
        (Err(_), Some(id)) => format!("Tape Backup '{id}' datastore '{}' failed", job.store,),
        (Err(_), None) => format!("Tape Backup datastore '{}' failed", job.store,),
    };

    send_job_status_mail(email, &subject, &text)?;

    Ok(())
}

/// Send email to a person to request a manual media change
pub fn send_load_media_email(
    changer: bool,
    device: &str,
    label_text: &str,
    to: &str,
    reason: Option<String>,
) -> Result<(), Error> {
    use std::fmt::Write as _;

    let device_type = if changer { "changer" } else { "drive" };

    let subject = format!("Load Media '{label_text}' request for {device_type} '{device}'");

    let mut text = String::new();

    if let Some(reason) = reason {
        let _ = write!(
            text,
            "The {device_type} has the wrong or no tape(s) inserted. Error:\n{reason}\n\n"
        );
    }

    if changer {
        text.push_str("Please insert the requested media into the changer.\n\n");
        let _ = writeln!(text, "Changer: {device}");
    } else {
        text.push_str("Please insert the requested media into the backup drive.\n\n");
        let _ = writeln!(text, "Drive: {device}");
    }
    let _ = writeln!(text, "Media: {label_text}");

    send_job_status_mail(to, &subject, &text)
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
    // update mails always go to the root@pam configured email..
    if let Some(email) = lookup_user_email(Userid::root_userid()) {
        let nodename = proxmox_sys::nodename();
        let subject = format!("New software packages available ({nodename})");

        let (fqdn, port) = get_server_url();

        let text = HANDLEBARS.render(
            "package_update_template",
            &json!({
                "fqdn": fqdn,
                "port": port,
                "updates": updates,
            }),
        )?;

        send_job_status_mail(&email, &subject, &text)?;
    }
    Ok(())
}

/// send email on certificate renewal failure.
pub fn send_certificate_renewal_mail(result: &Result<(), Error>) -> Result<(), Error> {
    let error: String = match result {
        Err(e) => e.to_string(),
        _ => return Ok(()),
    };

    if let Some(email) = lookup_user_email(Userid::root_userid()) {
        let (fqdn, port) = get_server_url();

        let text = HANDLEBARS.render(
            "certificate_renewal_err_template",
            &json!({
                "fqdn": fqdn,
                "port": port,
                "error": error,
            }),
        )?;

        let subject = "Could not renew certificate";

        send_job_status_mail(&email, subject, &text)?;
    }

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
pub fn lookup_datastore_notify_settings(store: &str) -> (Option<String>, DatastoreNotify) {
    let mut email = None;

    let notify = DatastoreNotify {
        gc: None,
        verify: None,
        sync: None,
        prune: None,
    };

    let (config, _digest) = match pbs_config::datastore::config() {
        Ok(result) => result,
        Err(_) => return (email, notify),
    };

    let config: DataStoreConfig = match config.lookup("datastore", store) {
        Ok(result) => result,
        Err(_) => return (email, notify),
    };

    email = match config.notify_user {
        Some(ref userid) => lookup_user_email(userid),
        None => lookup_user_email(Userid::root_userid()),
    };

    let notify_str = config.notify.unwrap_or_default();

    if let Ok(value) = DatastoreNotify::API_SCHEMA.parse_property_string(&notify_str) {
        if let Ok(notify) = serde_json::from_value(value) {
            return (email, notify);
        }
    }

    (email, notify)
}

// Handlerbar helper functions

fn handlebars_humam_bytes_helper(
    h: &Helper,
    _: &Handlebars,
    _: &Context,
    _rc: &mut RenderContext,
    out: &mut dyn Output,
) -> HelperResult {
    let param = h
        .param(0)
        .and_then(|v| v.value().as_u64())
        .ok_or_else(|| RenderError::new("human-bytes: param not found"))?;

    out.write(&HumanByte::from(param).to_string())?;

    Ok(())
}

fn handlebars_relative_percentage_helper(
    h: &Helper,
    _: &Handlebars,
    _: &Context,
    _rc: &mut RenderContext,
    out: &mut dyn Output,
) -> HelperResult {
    let param0 = h
        .param(0)
        .and_then(|v| v.value().as_f64())
        .ok_or_else(|| RenderError::new("relative-percentage: param0 not found"))?;
    let param1 = h
        .param(1)
        .and_then(|v| v.value().as_f64())
        .ok_or_else(|| RenderError::new("relative-percentage: param1 not found"))?;

    if param1 == 0.0 {
        out.write("-")?;
    } else {
        out.write(&format!("{:.2}%", (param0 * 100.0) / param1))?;
    }
    Ok(())
}

#[test]
fn test_template_register() {
    HANDLEBARS.get_helper("human-bytes").unwrap();
    HANDLEBARS.get_helper("relative-percentage").unwrap();

    assert!(HANDLEBARS.has_template("gc_ok_template"));
    assert!(HANDLEBARS.has_template("gc_err_template"));

    assert!(HANDLEBARS.has_template("verify_ok_template"));
    assert!(HANDLEBARS.has_template("verify_err_template"));

    assert!(HANDLEBARS.has_template("sync_ok_template"));
    assert!(HANDLEBARS.has_template("sync_err_template"));

    assert!(HANDLEBARS.has_template("tape_backup_ok_template"));
    assert!(HANDLEBARS.has_template("tape_backup_err_template"));

    assert!(HANDLEBARS.has_template("package_update_template"));

    assert!(HANDLEBARS.has_template("certificate_renewal_err_template"));
}
