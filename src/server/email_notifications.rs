use anyhow::Error;
use serde_json::json;

use handlebars::{Handlebars, Helper, Context, RenderError, RenderContext, Output, HelperResult};

use proxmox::tools::email::sendmail;

use crate::{
    config::verify::VerificationJobConfig,
    config::sync::SyncJobConfig,
    api2::types::{
        Userid,
        GarbageCollectionStatus,
    },
    tools::format::HumanByte,
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

"###;


const GC_ERR_TEMPLATE: &str = r###"

Datastore: {{datastore}}

Garbage collection failed: {{error}}

"###;

const VERIFY_OK_TEMPLATE: &str = r###"

Job ID:    {{job.id}}
Datastore: {{job.store}}

Verification successful.

"###;

const VERIFY_ERR_TEMPLATE: &str = r###"

Job ID:    {{job.id}}
Datastore: {{job.store}}

Verification failed on these snapshots:

{{#each errors}}
  {{this}}
{{/each}}

"###;

const SYNC_OK_TEMPLATE: &str = r###"

Job ID:       {{job.id}}
Datastore:    {{job.store}}
Remote:       {{job.remote}}
Remote Store: {{job.remote-store}}

Synchronization successful.

"###;

const SYNC_ERR_TEMPLATE: &str = r###"

Job ID:       {{job.id}}
Datastore:    {{job.store}}
Remote:       {{job.remote}}
Remote Store: {{job.remote-store}}

Synchronization failed: {{error}}

"###;

lazy_static::lazy_static!{

    static ref HANDLEBARS: Handlebars<'static> = {
        let mut hb = Handlebars::new();

        hb.set_strict_mode(true);

        hb.register_helper("human-bytes", Box::new(handlebars_humam_bytes_helper));
        hb.register_helper("relative-percentage", Box::new(handlebars_relative_percentage_helper));

        hb.register_template_string("gc_ok_template", GC_OK_TEMPLATE).unwrap();
        hb.register_template_string("gc_err_template", GC_ERR_TEMPLATE).unwrap();

        hb.register_template_string("verify_ok_template", VERIFY_OK_TEMPLATE).unwrap();
        hb.register_template_string("verify_err_template", VERIFY_ERR_TEMPLATE).unwrap();

        hb.register_template_string("sync_ok_template", SYNC_OK_TEMPLATE).unwrap();
        hb.register_template_string("sync_err_template", SYNC_ERR_TEMPLATE).unwrap();

        hb
    };
}

fn send_job_status_mail(
    email: &str,
    subject: &str,
    text: &str,
) -> Result<(), Error> {

    // Note: OX has serious problems displaying text mails,
    // so we include html as well
    let html = format!("<html><body><pre>\n{}\n<pre>", handlebars::html_escape(text));

    let nodename = proxmox::tools::nodename();

    let author = format!("Proxmox Backup Server - {}", nodename);

    sendmail(
        &[email],
        &subject,
        Some(&text),
        Some(&html),
        None,
        Some(&author),
    )?;

    Ok(())
}

pub fn send_gc_status(
    email: &str,
    datastore: &str,
    status: &GarbageCollectionStatus,
    result: &Result<(), Error>,
) -> Result<(), Error> {

    let text = match result {
        Ok(()) => {
            let deduplication_factor = if status.disk_bytes > 0 {
                (status.index_data_bytes as f64)/(status.disk_bytes as f64)
            } else {
                1.0
            };

            let data = json!({
                "status": status,
                "datastore": datastore,
                "deduplication-factor": format!("{:.2}", deduplication_factor),
            });

            HANDLEBARS.render("gc_ok_template", &data)?
        }
        Err(err) => {
            let data = json!({
                "error": err.to_string(),
                "datastore": datastore,
            });
            HANDLEBARS.render("gc_err_template", &data)?
        }
    };

    let subject = match result {
        Ok(()) => format!(
            "Garbage Collect Datastore '{}' successful",
            datastore,
        ),
        Err(_) => format!(
            "Garbage Collect Datastore '{}' failed",
            datastore,
        ),
    };

    send_job_status_mail(email, &subject, &text)?;

    Ok(())
}

pub fn send_verify_status(
    email: &str,
    job: VerificationJobConfig,
    result: &Result<Vec<String>, Error>,
) -> Result<(), Error> {


    let text = match result {
        Ok(errors) if errors.is_empty() => {
            let data = json!({ "job": job });
            HANDLEBARS.render("verify_ok_template", &data)?
        }
        Ok(errors) => {
            let data = json!({ "job": job, "errors": errors });
            HANDLEBARS.render("verify_err_template", &data)?
        }
        Err(_) => {
            // aboreted job - do not send any email
            return Ok(());
        }
    };

    let subject = match result {
        Ok(errors) if errors.is_empty() => format!(
            "Verify Datastore '{}' successful",
            job.store,
        ),
        _ => format!(
            "Verify Datastore '{}' failed",
            job.store,
        ),
    };

    send_job_status_mail(email, &subject, &text)?;

    Ok(())
}

pub fn send_sync_status(
    email: &str,
    job: &SyncJobConfig,
    result: &Result<(), Error>,
) -> Result<(), Error> {

    let text = match result {
        Ok(()) => {
            let data = json!({ "job": job });
            HANDLEBARS.render("sync_ok_template", &data)?
        }
        Err(err) => {
            let data = json!({ "job": job, "error": err.to_string() });
            HANDLEBARS.render("sync_err_template", &data)?
        }
    };

    let subject = match result {
        Ok(()) => format!(
            "Sync remote '{}' datastore '{}' successful",
            job.remote,
            job.remote_store,
        ),
        Err(_) => format!(
            "Sync remote '{}' datastore '{}' failed",
            job.remote,
            job.remote_store,
        ),
    };

    send_job_status_mail(email, &subject, &text)?;

    Ok(())
}

/// Lookup users email address
///
/// For "backup@pam", this returns the address from "root@pam".
pub fn lookup_user_email(userid: &Userid) -> Option<String> {

    use crate::config::user::{self, User};

    if userid == Userid::backup_userid() {
        return lookup_user_email(Userid::root_userid());
    }

    if let Ok(user_config) = user::cached_config() {
        if let Ok(user) = user_config.lookup::<User>("user", userid.as_str()) {
            return user.email.clone();
        }
    }

    None
}

// Handlerbar helper functions

fn handlebars_humam_bytes_helper(
    h: &Helper,
    _: &Handlebars,
    _: &Context,
    _rc: &mut RenderContext,
    out: &mut dyn Output
) -> HelperResult {
    let param = h.param(0).map(|v| v.value().as_u64())
        .flatten()
        .ok_or(RenderError::new("human-bytes: param not found"))?;

    out.write(&HumanByte::from(param).to_string())?;

    Ok(())
}

fn handlebars_relative_percentage_helper(
    h: &Helper,
    _: &Handlebars,
    _: &Context,
    _rc: &mut RenderContext,
    out: &mut dyn Output
) -> HelperResult {
    let param0 = h.param(0).map(|v| v.value().as_f64())
        .flatten()
        .ok_or(RenderError::new("relative-percentage: param0 not found"))?;
    let param1 = h.param(1).map(|v| v.value().as_f64())
        .flatten()
        .ok_or(RenderError::new("relative-percentage: param1 not found"))?;

    if param1 == 0.0 {
        out.write("-")?;
    } else {
        out.write(&format!("{:.2}%", (param0*100.0)/param1))?;
    }
    Ok(())
}
