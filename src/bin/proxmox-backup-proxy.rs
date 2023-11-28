use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{bail, format_err, Context, Error};
use futures::*;
use http::request::Parts;
use http::Response;
use hyper::header;
use hyper::{Body, StatusCode};
use url::form_urlencoded;

use openssl::ssl::SslAcceptor;
use serde_json::{json, Value};

use proxmox_lang::try_block;
use proxmox_metrics::MetricsData;
use proxmox_router::{RpcEnvironment, RpcEnvironmentType};
use proxmox_sys::fs::{CreateOptions, FileSystemInformation};
use proxmox_sys::linux::procfs::{Loadavg, ProcFsMemInfo, ProcFsNetDev, ProcFsStat};
use proxmox_sys::logrotate::LogRotate;
use proxmox_sys::{task_log, task_warn};

use pbs_datastore::DataStore;

use proxmox_rest_server::{
    cleanup_old_tasks, cookie_from_header, rotate_task_log_archive, ApiConfig, Redirector,
    RestEnvironment, RestServer, WorkerTask,
};

use proxmox_backup::rrd_cache::{
    initialize_rrd_cache, rrd_sync_journal, rrd_update_derive, rrd_update_gauge,
};
use proxmox_backup::{
    server::{
        auth::check_pbs_auth,
        jobstate::{self, Job},
    },
    tools::disks::BlockDevStat,
    traffic_control_cache::{SharedRateLimit, TRAFFIC_CONTROL_CACHE},
};

use pbs_buildcfg::configdir;
use proxmox_time::CalendarEvent;

use pbs_api_types::{
    Authid, DataStoreConfig, Operation, PruneJobConfig, SyncJobConfig, TapeBackupJobConfig,
    VerificationJobConfig,
};

use proxmox_rest_server::daemon;

use proxmox_backup::auth_helpers::*;
use proxmox_backup::server;
use proxmox_backup::tools::{
    disks::{zfs_dataset_stats, DiskManage},
    PROXMOX_BACKUP_TCP_KEEPALIVE_TIME,
};

use proxmox_backup::api2::pull::do_sync_job;
use proxmox_backup::api2::tape::backup::do_tape_backup_job;
use proxmox_backup::server::do_prune_job;
use proxmox_backup::server::do_verification_job;

fn main() -> Result<(), Error> {
    pbs_tools::setup_libc_malloc_opts();

    proxmox_backup::tools::setup_safe_path_env();

    let backup_uid = pbs_config::backup_user()?.uid;
    let backup_gid = pbs_config::backup_group()?.gid;
    let running_uid = nix::unistd::Uid::effective();
    let running_gid = nix::unistd::Gid::effective();

    if running_uid != backup_uid || running_gid != backup_gid {
        bail!(
            "proxy not running as backup user or group (got uid {running_uid} gid {running_gid})"
        );
    }

    proxmox_async::runtime::main(run())
}

/// check for a cookie with the user-preferred language, fallback to the config one if not set or
/// not existing
fn get_language(headers: &http::HeaderMap) -> String {
    let exists = |l: &str| Path::new(&format!("/usr/share/pbs-i18n/pbs-lang-{l}.js")).exists();

    match cookie_from_header(headers, "PBSLangCookie") {
        Some(cookie_lang) if exists(&cookie_lang) => cookie_lang,
        _ => match proxmox_backup::config::node::config().map(|(cfg, _)| cfg.default_lang) {
            Ok(Some(default_lang)) if exists(&default_lang) => default_lang,
            _ => String::from(""),
        },
    }
}

fn get_theme(headers: &http::HeaderMap) -> String {
    let exists = |t: &str| {
        t.len() < 32
            && !t.contains('/')
            && Path::new(&format!(
                "/usr/share/javascript/proxmox-widget-toolkit/themes/theme-{t}.css"
            ))
            .exists()
    };

    match cookie_from_header(headers, "PBSThemeCookie") {
        Some(theme) if theme == "crisp" => String::from(""),
        Some(theme) if exists(&theme) => theme,
        _ => String::from("auto"),
    }
}

async fn get_index_future(env: RestEnvironment, parts: Parts) -> Response<Body> {
    let auth_id = env.get_auth_id();
    let api = env.api_config();

    // fixme: make all IO async

    let (userid, csrf_token) = match auth_id {
        Some(auth_id) => {
            let auth_id = auth_id.parse::<Authid>();
            match auth_id {
                Ok(auth_id) if !auth_id.is_token() => {
                    let userid = auth_id.user().clone();
                    let new_csrf_token = assemble_csrf_prevention_token(csrf_secret(), &userid);
                    (Some(userid), Some(new_csrf_token))
                }
                _ => (None, None),
            }
        }
        None => (None, None),
    };

    let nodename = proxmox_sys::nodename();
    let user = userid.as_ref().map(|u| u.as_str()).unwrap_or("");

    let csrf_token = csrf_token.unwrap_or_else(|| String::from(""));

    let mut debug = false;
    let mut template_file = "index";

    if let Some(query_str) = parts.uri.query() {
        for (k, v) in form_urlencoded::parse(query_str.as_bytes()).into_owned() {
            if k == "debug" && v != "0" && v != "false" {
                debug = true;
            } else if k == "console" {
                template_file = "console";
            }
        }
    }

    let theme = get_theme(&parts.headers);

    let data = json!({
        "NodeName": nodename,
        "UserName": user,
        "CSRFPreventionToken": csrf_token,
        "language": get_language(&parts.headers),
        "theme": theme,
        "auto": theme == "auto",
        "debug": debug,
    });

    let (ct, index) = match api.render_template(template_file, &data) {
        Ok(index) => ("text/html", index),
        Err(err) => ("text/plain", format!("Error rendering template: {err}")),
    };

    let mut resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, ct)
        .body(index.into())
        .unwrap();

    if let Some(userid) = userid {
        resp.extensions_mut().insert(Authid::from((userid, None)));
    }

    resp
}

async fn run() -> Result<(), Error> {
    // Note: To debug early connection error use
    // PROXMOX_DEBUG=1 ./target/release/proxmox-backup-proxy
    let debug = std::env::var("PROXMOX_DEBUG").is_ok();

    if let Err(err) = syslog::init(
        syslog::Facility::LOG_DAEMON,
        if debug {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        },
        Some("proxmox-backup-proxy"),
    ) {
        bail!("unable to inititialize syslog - {err}");
    }

    proxmox_backup::auth_helpers::setup_auth_context(false);

    let rrd_cache = initialize_rrd_cache()?;
    rrd_cache.apply_journal()?;

    let mut indexpath = PathBuf::from(pbs_buildcfg::JS_DIR);
    indexpath.push("index.hbs");

    let mut config = ApiConfig::new(pbs_buildcfg::JS_DIR, RpcEnvironmentType::PUBLIC)
        .index_handler_func(|e, p| Box::pin(get_index_future(e, p)))
        .auth_handler_func(|h, m| Box::pin(check_pbs_auth(h, m)))
        .register_template("index", &indexpath)?
        .register_template("console", "/usr/share/pve-xtermjs/index.html.hbs")?
        .default_api2_handler(&proxmox_backup::api2::ROUTER)
        .aliases([
            ("novnc", "/usr/share/novnc-pve"),
            ("extjs", "/usr/share/javascript/extjs"),
            ("qrcodejs", "/usr/share/javascript/qrcodejs"),
            ("fontawesome", "/usr/share/fonts-font-awesome"),
            ("xtermjs", "/usr/share/pve-xtermjs"),
            ("locale", "/usr/share/pbs-i18n"),
            (
                "widgettoolkit",
                "/usr/share/javascript/proxmox-widget-toolkit",
            ),
            ("docs", "/usr/share/doc/proxmox-backup/html"),
        ]);

    let backup_user = pbs_config::backup_user()?;
    let mut command_sock = proxmox_rest_server::CommandSocket::new(
        proxmox_rest_server::our_ctrl_sock(),
        backup_user.gid,
    );

    let dir_opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);
    let file_opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);

    config = config
        .enable_access_log(
            pbs_buildcfg::API_ACCESS_LOG_FN,
            Some(dir_opts.clone()),
            Some(file_opts.clone()),
            &mut command_sock,
        )?
        .enable_auth_log(
            pbs_buildcfg::API_AUTH_LOG_FN,
            Some(dir_opts.clone()),
            Some(file_opts.clone()),
            &mut command_sock,
        )?;

    let rest_server = RestServer::new(config);
    let redirector = Redirector::new();
    proxmox_rest_server::init_worker_tasks(
        pbs_buildcfg::PROXMOX_BACKUP_LOG_DIR_M!().into(),
        file_opts.clone(),
    )?;

    //openssl req -x509 -newkey rsa:4096 -keyout /etc/proxmox-backup/proxy.key -out /etc/proxmox-backup/proxy.pem -nodes

    // we build the initial acceptor here as we cannot start if this fails
    let acceptor = make_tls_acceptor()?;
    let acceptor = Arc::new(Mutex::new(acceptor));

    // to renew the acceptor we just add a command-socket handler
    command_sock.register_command("reload-certificate".to_string(), {
        let acceptor = Arc::clone(&acceptor);
        move |_value| -> Result<_, Error> {
            log::info!("reloading certificate");
            match make_tls_acceptor() {
                Err(err) => log::error!("error reloading certificate: {err}"),
                Ok(new_acceptor) => {
                    let mut guard = acceptor.lock().unwrap();
                    *guard = new_acceptor;
                }
            }
            Ok(Value::Null)
        }
    })?;

    // to remove references for not configured datastores
    command_sock.register_command("datastore-removed".to_string(), |_value| {
        if let Err(err) = DataStore::remove_unused_datastores() {
            log::error!("could not refresh datastores: {err}");
        }
        Ok(Value::Null)
    })?;

    let connections = proxmox_rest_server::connection::AcceptBuilder::new()
        .debug(debug)
        .rate_limiter_lookup(Arc::new(lookup_rate_limiter))
        .tcp_keepalive_time(PROXMOX_BACKUP_TCP_KEEPALIVE_TIME);

    let server = daemon::create_daemon(
        ([0, 0, 0, 0, 0, 0, 0, 0], 8007).into(),
        move |listener| {
            let (secure_connections, insecure_connections) =
                connections.accept_tls_optional(listener, acceptor);

            Ok(async {
                daemon::systemd_notify(daemon::SystemdNotify::Ready)?;

                let secure_server = hyper::Server::builder(secure_connections)
                    .serve(rest_server)
                    .with_graceful_shutdown(proxmox_rest_server::shutdown_future())
                    .map_err(Error::from);

                let insecure_server = hyper::Server::builder(insecure_connections)
                    .serve(redirector)
                    .with_graceful_shutdown(proxmox_rest_server::shutdown_future())
                    .map_err(Error::from);

                let (secure_res, insecure_res) =
                    try_join!(tokio::spawn(secure_server), tokio::spawn(insecure_server))
                        .context("failed to complete REST server task")?;

                let results = [secure_res, insecure_res];

                if results.iter().any(Result::is_err) {
                    let cat_errors = results
                        .into_iter()
                        .filter_map(|res| res.err().map(|err| err.to_string()))
                        .collect::<Vec<_>>()
                        .join("\n");

                    bail!(cat_errors);
                }

                Ok(())
            })
        },
        Some(pbs_buildcfg::PROXMOX_BACKUP_PROXY_PID_FN),
    );

    proxmox_rest_server::write_pid(pbs_buildcfg::PROXMOX_BACKUP_PROXY_PID_FN)?;

    let init_result: Result<(), Error> = try_block!({
        proxmox_rest_server::register_task_control_commands(&mut command_sock)?;
        command_sock.spawn()?;
        proxmox_rest_server::catch_shutdown_signal()?;
        proxmox_rest_server::catch_reload_signal()?;
        Ok(())
    });

    if let Err(err) = init_result {
        bail!("unable to start daemon - {err}");
    }

    // stop gap for https://github.com/tokio-rs/tokio/issues/4730 where the thread holding the
    // IO-driver may block progress completely if it starts polling its own tasks (blocks).
    // So, trigger a notify to parked threads, as we're immediately ready the woken up thread will
    // acquire the IO driver, if blocked, before going to sleep, which allows progress again
    // TODO: remove once tokio solves this at their level (see proposals in linked comments)
    let rt_handle = tokio::runtime::Handle::current();
    std::thread::spawn(move || loop {
        rt_handle.spawn(std::future::ready(()));
        std::thread::sleep(Duration::from_secs(3));
    });

    start_task_scheduler();
    start_stat_generator();
    start_traffic_control_updater();

    server.await?;
    log::info!("server shutting down, waiting for active workers to complete");
    proxmox_rest_server::last_worker_future().await?;
    log::info!("done - exit server");

    Ok(())
}

fn make_tls_acceptor() -> Result<SslAcceptor, Error> {
    let key_path = configdir!("/proxy.key");
    let cert_path = configdir!("/proxy.pem");

    let (config, _) = proxmox_backup::config::node::config()?;
    let ciphers_tls_1_3 = config.ciphers_tls_1_3;
    let ciphers_tls_1_2 = config.ciphers_tls_1_2;

    let mut acceptor = proxmox_rest_server::connection::TlsAcceptorBuilder::new()
        .certificate_paths_pem(key_path, cert_path);

    //let mut acceptor = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls()).unwrap();
    if let Some(ciphers) = ciphers_tls_1_3.as_deref() {
        acceptor = acceptor.cipher_suites(ciphers.to_string());
    }
    if let Some(ciphers) = ciphers_tls_1_2.as_deref() {
        acceptor = acceptor.cipher_list(ciphers.to_string());
    }

    acceptor.build()
}

fn start_stat_generator() {
    let abort_future = proxmox_rest_server::shutdown_future();
    let future = Box::pin(run_stat_generator());
    let task = futures::future::select(future, abort_future);
    tokio::spawn(task.map(|_| ()));
}

fn start_task_scheduler() {
    let abort_future = proxmox_rest_server::shutdown_future();
    let future = Box::pin(run_task_scheduler());
    let task = futures::future::select(future, abort_future);
    tokio::spawn(task.map(|_| ()));
}

fn start_traffic_control_updater() {
    let abort_future = proxmox_rest_server::shutdown_future();
    let future = Box::pin(run_traffic_control_updater());
    let task = futures::future::select(future, abort_future);
    tokio::spawn(task.map(|_| ()));
}

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn next_minute() -> Instant {
    let now = SystemTime::now();
    let epoch_now = match now.duration_since(UNIX_EPOCH) {
        Ok(d) => d,
        Err(err) => {
            eprintln!("task scheduler: compute next minute alignment failed - {err}");
            return Instant::now() + Duration::from_secs(60);
        }
    };
    let epoch_next = Duration::from_secs((epoch_now.as_secs() / 60 + 1) * 60);
    Instant::now() + epoch_next - epoch_now
}

async fn run_task_scheduler() {
    loop {
        // sleep first to align to next minute boundary for first round
        let delay_target = next_minute();
        tokio::time::sleep_until(tokio::time::Instant::from_std(delay_target)).await;

        match schedule_tasks().catch_unwind().await {
            Err(panic) => match panic.downcast::<&str>() {
                Ok(msg) => eprintln!("task scheduler panic: {msg}"),
                Err(_) => eprintln!("task scheduler panic - unknown type"),
            },
            Ok(Err(err)) => eprintln!("task scheduler failed - {err:?}"),
            Ok(Ok(_)) => {}
        }
    }
}

async fn schedule_tasks() -> Result<(), Error> {
    schedule_datastore_garbage_collection().await;
    schedule_datastore_prune_jobs().await;
    schedule_datastore_sync_jobs().await;
    schedule_datastore_verify_jobs().await;
    schedule_tape_backup_jobs().await;
    schedule_task_log_rotate().await;

    Ok(())
}

async fn schedule_datastore_garbage_collection() {
    let config = match pbs_config::datastore::config() {
        Err(err) => {
            eprintln!("unable to read datastore config - {err}");
            return;
        }
        Ok((config, _digest)) => config,
    };

    for (store, (_, store_config)) in config.sections {
        let store_config: DataStoreConfig = match serde_json::from_value(store_config) {
            Ok(c) => c,
            Err(err) => {
                eprintln!("datastore config from_value failed - {err}");
                continue;
            }
        };

        let event_str = match store_config.gc_schedule {
            Some(event_str) => event_str,
            None => continue,
        };

        let event: CalendarEvent = match event_str.parse() {
            Ok(event) => event,
            Err(err) => {
                eprintln!("unable to parse schedule '{event_str}' - {err}");
                continue;
            }
        };

        {
            // limit datastore scope due to Op::Lookup
            let datastore = match DataStore::lookup_datastore(&store, Some(Operation::Lookup)) {
                Ok(datastore) => datastore,
                Err(err) => {
                    eprintln!("lookup_datastore failed - {err}");
                    continue;
                }
            };

            if datastore.garbage_collection_running() {
                continue;
            }
        }

        let worker_type = "garbage_collection";

        let last = match jobstate::last_run_time(worker_type, &store) {
            Ok(time) => time,
            Err(err) => {
                eprintln!("could not get last run time of {worker_type} {store}: {err}");
                continue;
            }
        };

        let next = match event.compute_next_event(last) {
            Ok(Some(next)) => next,
            Ok(None) => continue,
            Err(err) => {
                eprintln!("compute_next_event for '{event_str}' failed - {err}");
                continue;
            }
        };

        let now = proxmox_time::epoch_i64();

        if next > now {
            continue;
        }

        let job = match Job::new(worker_type, &store) {
            Ok(job) => job,
            Err(_) => continue, // could not get lock
        };

        let datastore = match DataStore::lookup_datastore(&store, Some(Operation::Write)) {
            Ok(datastore) => datastore,
            Err(err) => {
                log::warn!("skipping scheduled GC on {store}, could look it up - {err}");
                continue;
            }
        };

        let auth_id = Authid::root_auth_id();

        if let Err(err) = crate::server::do_garbage_collection_job(
            job,
            datastore,
            auth_id,
            Some(event_str),
            false,
        ) {
            eprintln!("unable to start garbage collection job on datastore {store} - {err}");
        }
    }
}

async fn schedule_datastore_prune_jobs() {
    let config = match pbs_config::prune::config() {
        Err(err) => {
            eprintln!("unable to read prune job config - {err}");
            return;
        }
        Ok((config, _digest)) => config,
    };
    for (job_id, (_, job_config)) in config.sections {
        let job_config: PruneJobConfig = match serde_json::from_value(job_config) {
            Ok(c) => c,
            Err(err) => {
                eprintln!("prune job config from_value failed - {err}");
                continue;
            }
        };

        if job_config.disable {
            continue;
        }

        if !job_config.options.keeps_something() {
            continue; // no 'keep' values set, keep all
        }

        let worker_type = "prunejob";
        let auth_id = Authid::root_auth_id().clone();
        if check_schedule(worker_type, &job_config.schedule, &job_id) {
            let job = match Job::new(worker_type, &job_id) {
                Ok(job) => job,
                Err(_) => continue, // could not get lock
            };
            if let Err(err) = do_prune_job(
                job,
                job_config.options,
                job_config.store,
                &auth_id,
                Some(job_config.schedule),
            ) {
                eprintln!("unable to start datastore prune job {job_id} - {err}");
            }
        };
    }
}

async fn schedule_datastore_sync_jobs() {
    let config = match pbs_config::sync::config() {
        Err(err) => {
            eprintln!("unable to read sync job config - {err}");
            return;
        }
        Ok((config, _digest)) => config,
    };

    for (job_id, (_, job_config)) in config.sections {
        let job_config: SyncJobConfig = match serde_json::from_value(job_config) {
            Ok(c) => c,
            Err(err) => {
                eprintln!("sync job config from_value failed - {err}");
                continue;
            }
        };

        let event_str = match job_config.schedule {
            Some(ref event_str) => event_str.clone(),
            None => continue,
        };

        let worker_type = "syncjob";
        if check_schedule(worker_type, &event_str, &job_id) {
            let job = match Job::new(worker_type, &job_id) {
                Ok(job) => job,
                Err(_) => continue, // could not get lock
            };

            let auth_id = Authid::root_auth_id().clone();
            if let Err(err) = do_sync_job(job, job_config, &auth_id, Some(event_str), false) {
                eprintln!("unable to start datastore sync job {job_id} - {err}");
            }
        };
    }
}

async fn schedule_datastore_verify_jobs() {
    let config = match pbs_config::verify::config() {
        Err(err) => {
            eprintln!("unable to read verification job config - {err}");
            return;
        }
        Ok((config, _digest)) => config,
    };
    for (job_id, (_, job_config)) in config.sections {
        let job_config: VerificationJobConfig = match serde_json::from_value(job_config) {
            Ok(c) => c,
            Err(err) => {
                eprintln!("verification job config from_value failed - {err}");
                continue;
            }
        };
        let event_str = match job_config.schedule {
            Some(ref event_str) => event_str.clone(),
            None => continue,
        };

        let worker_type = "verificationjob";
        let auth_id = Authid::root_auth_id().clone();
        if check_schedule(worker_type, &event_str, &job_id) {
            let job = match Job::new(worker_type, &job_id) {
                Ok(job) => job,
                Err(_) => continue, // could not get lock
            };
            if let Err(err) = do_verification_job(job, job_config, &auth_id, Some(event_str), false)
            {
                eprintln!("unable to start datastore verification job {job_id} - {err}");
            }
        };
    }
}

async fn schedule_tape_backup_jobs() {
    let config = match pbs_config::tape_job::config() {
        Err(err) => {
            eprintln!("unable to read tape job config - {err}");
            return;
        }
        Ok((config, _digest)) => config,
    };
    for (job_id, (_, job_config)) in config.sections {
        let job_config: TapeBackupJobConfig = match serde_json::from_value(job_config) {
            Ok(c) => c,
            Err(err) => {
                eprintln!("tape backup job config from_value failed - {err}");
                continue;
            }
        };
        let event_str = match job_config.schedule {
            Some(ref event_str) => event_str.clone(),
            None => continue,
        };

        let worker_type = "tape-backup-job";
        let auth_id = Authid::root_auth_id().clone();
        if check_schedule(worker_type, &event_str, &job_id) {
            let job = match Job::new(worker_type, &job_id) {
                Ok(job) => job,
                Err(_) => continue, // could not get lock
            };
            if let Err(err) =
                do_tape_backup_job(job, job_config.setup, &auth_id, Some(event_str), false)
            {
                eprintln!("unable to start tape backup job {job_id} - {err}");
            }
        };
    }
}

async fn schedule_task_log_rotate() {
    let worker_type = "logrotate";
    let job_id = "access-log_and_task-archive";

    // schedule daily at 00:00 like normal logrotate
    let schedule = "00:00";

    if !check_schedule(worker_type, schedule, job_id) {
        // if we never ran the rotation, schedule instantly
        match jobstate::JobState::load(worker_type, job_id) {
            Ok(jobstate::JobState::Created { .. }) => {}
            _ => return,
        }
    }

    let mut job = match Job::new(worker_type, job_id) {
        Ok(job) => job,
        Err(_) => return, // could not get lock
    };

    if let Err(err) = WorkerTask::new_thread(
        worker_type,
        None,
        Authid::root_auth_id().to_string(),
        false,
        move |worker| {
            job.start(&worker.upid().to_string())?;
            task_log!(worker, "starting task log rotation");

            let result = try_block!({
                let max_size = 512 * 1024 - 1; // an entry has ~ 100b, so > 5000 entries/file
                let max_files = 20; // times twenty files gives > 100000 task entries

                let max_days = proxmox_backup::config::node::config()
                    .map(|(cfg, _)| cfg.task_log_max_days)
                    .ok()
                    .flatten();

                let user = pbs_config::backup_user()?;
                let options = proxmox_sys::fs::CreateOptions::new()
                    .owner(user.uid)
                    .group(user.gid);

                let has_rotated = rotate_task_log_archive(
                    max_size,
                    true,
                    Some(max_files),
                    max_days,
                    Some(options.clone()),
                )?;

                if has_rotated {
                    task_log!(worker, "task log archive was rotated");
                } else {
                    task_log!(worker, "task log archive was not rotated");
                }

                let max_size = 32 * 1024 * 1024 - 1;
                let max_files = 14;

                let mut logrotate = LogRotate::new(
                    pbs_buildcfg::API_ACCESS_LOG_FN,
                    true,
                    Some(max_files),
                    Some(options.clone()),
                )?;

                if logrotate.rotate(max_size)? {
                    println!("rotated access log, telling daemons to re-open log file");
                    proxmox_async::runtime::block_on(command_reopen_access_logfiles())?;
                    task_log!(worker, "API access log was rotated");
                } else {
                    task_log!(worker, "API access log was not rotated");
                }

                let mut logrotate = LogRotate::new(
                    pbs_buildcfg::API_AUTH_LOG_FN,
                    true,
                    Some(max_files),
                    Some(options),
                )?;

                if logrotate.rotate(max_size)? {
                    println!("rotated auth log, telling daemons to re-open log file");
                    proxmox_async::runtime::block_on(command_reopen_auth_logfiles())?;
                    task_log!(worker, "API authentication log was rotated");
                } else {
                    task_log!(worker, "API authentication log was not rotated");
                }

                if has_rotated {
                    task_log!(worker, "cleaning up old task logs");
                    if let Err(err) = cleanup_old_tasks(&worker, true) {
                        task_warn!(worker, "could not completely cleanup old tasks: {err}");
                    }
                }

                Ok(())
            });

            let status = worker.create_state(&result);

            if let Err(err) = job.finish(status) {
                eprintln!("could not finish job state for {worker_type}: {err}");
            }

            result
        },
    ) {
        eprintln!("unable to start task log rotation: {err}");
    }
}

async fn command_reopen_access_logfiles() -> Result<(), Error> {
    // only care about the most recent daemon instance for each, proxy & api, as other older ones
    // should not respond to new requests anyway, but only finish their current one and then exit.
    let sock = proxmox_rest_server::our_ctrl_sock();
    let f1 =
        proxmox_rest_server::send_raw_command(sock, "{\"command\":\"api-access-log-reopen\"}\n");

    let pid = proxmox_rest_server::read_pid(pbs_buildcfg::PROXMOX_BACKUP_API_PID_FN)?;
    let sock = proxmox_rest_server::ctrl_sock_from_pid(pid);
    let f2 =
        proxmox_rest_server::send_raw_command(sock, "{\"command\":\"api-access-log-reopen\"}\n");

    match futures::join!(f1, f2) {
        (Err(e1), Err(e2)) => Err(format_err!(
            "reopen commands failed, proxy: {e1}; api: {e2}"
        )),
        (Err(e1), Ok(_)) => Err(format_err!("reopen commands failed, proxy: {e1}")),
        (Ok(_), Err(e2)) => Err(format_err!("reopen commands failed, api: {e2}")),
        _ => Ok(()),
    }
}

async fn command_reopen_auth_logfiles() -> Result<(), Error> {
    // only care about the most recent daemon instance for each, proxy & api, as other older ones
    // should not respond to new requests anyway, but only finish their current one and then exit.
    let sock = proxmox_rest_server::our_ctrl_sock();
    let f1 = proxmox_rest_server::send_raw_command(sock, "{\"command\":\"api-auth-log-reopen\"}\n");

    let pid = proxmox_rest_server::read_pid(pbs_buildcfg::PROXMOX_BACKUP_API_PID_FN)?;
    let sock = proxmox_rest_server::ctrl_sock_from_pid(pid);
    let f2 = proxmox_rest_server::send_raw_command(sock, "{\"command\":\"api-auth-log-reopen\"}\n");

    match futures::join!(f1, f2) {
        (Err(e1), Err(e2)) => Err(format_err!(
            "reopen commands failed, proxy: {e1}; api: {e2}"
        )),
        (Err(e1), Ok(_)) => Err(format_err!("reopen commands failed, proxy: {e1}")),
        (Ok(_), Err(e2)) => Err(format_err!("reopen commands failed, api: {e2}")),
        _ => Ok(()),
    }
}

async fn run_stat_generator() {
    loop {
        let delay_target = Instant::now() + Duration::from_secs(10);

        let stats = match tokio::task::spawn_blocking(|| {
            let hoststats = collect_host_stats_sync();
            let (hostdisk, datastores) = collect_disk_stats_sync();
            Arc::new((hoststats, hostdisk, datastores))
        })
        .await
        {
            Ok(res) => res,
            Err(err) => {
                log::error!("collecting host stats panicked: {err}");
                tokio::time::sleep_until(tokio::time::Instant::from_std(delay_target)).await;
                continue;
            }
        };

        let rrd_future = tokio::task::spawn_blocking({
            let stats = Arc::clone(&stats);
            move || {
                rrd_update_host_stats_sync(&stats.0, &stats.1, &stats.2);
                rrd_sync_journal();
            }
        });

        let metrics_future = send_data_to_metric_servers(stats);

        let (rrd_res, metrics_res) = join!(rrd_future, metrics_future);
        if let Err(err) = rrd_res {
            log::error!("rrd update panicked: {err}");
        }
        if let Err(err) = metrics_res {
            log::error!("error during metrics sending: {err}");
        }

        tokio::time::sleep_until(tokio::time::Instant::from_std(delay_target)).await;
    }
}

async fn send_data_to_metric_servers(
    stats: Arc<(HostStats, DiskStat, Vec<DiskStat>)>,
) -> Result<(), Error> {
    let (config, _digest) = pbs_config::metrics::config()?;
    let channel_list = get_metric_server_connections(config)?;

    if channel_list.is_empty() {
        return Ok(());
    }

    let ctime = proxmox_time::epoch_i64();
    let nodename = proxmox_sys::nodename();

    let mut values = Vec::new();

    let mut cpuvalue = match &stats.0.proc {
        Some(stat) => serde_json::to_value(stat)?,
        None => json!({}),
    };

    if let Some(loadavg) = &stats.0.load {
        cpuvalue["avg1"] = Value::from(loadavg.0);
        cpuvalue["avg5"] = Value::from(loadavg.1);
        cpuvalue["avg15"] = Value::from(loadavg.2);
    }

    values.push(Arc::new(
        MetricsData::new("cpustat", ctime, cpuvalue)?
            .tag("object", "host")
            .tag("host", nodename),
    ));

    if let Some(stat) = &stats.0.meminfo {
        values.push(Arc::new(
            MetricsData::new("memory", ctime, stat)?
                .tag("object", "host")
                .tag("host", nodename),
        ));
    }

    if let Some(netdev) = &stats.0.net {
        for item in netdev {
            values.push(Arc::new(
                MetricsData::new("nics", ctime, item)?
                    .tag("object", "host")
                    .tag("host", nodename)
                    .tag("instance", item.device.clone()),
            ));
        }
    }

    values.push(Arc::new(
        MetricsData::new("blockstat", ctime, stats.1.to_value())?
            .tag("object", "host")
            .tag("host", nodename),
    ));

    for datastore in stats.2.iter() {
        values.push(Arc::new(
            MetricsData::new("blockstat", ctime, datastore.to_value())?
                .tag("object", "host")
                .tag("host", nodename)
                .tag("datastore", datastore.name.clone()),
        ));
    }

    // we must have a concrete functions, because the inferred lifetime from a
    // closure is not general enough for the tokio::spawn call we are in here...
    fn map_fn(item: &(proxmox_metrics::Metrics, String)) -> &proxmox_metrics::Metrics {
        &item.0
    }

    let results =
        proxmox_metrics::send_data_to_channels(&values, channel_list.iter().map(map_fn)).await;
    for (res, name) in results
        .into_iter()
        .zip(channel_list.iter().map(|(_, name)| name))
    {
        if let Err(err) = res {
            log::error!("error sending into channel of {name}: {err}");
        }
    }

    futures::future::join_all(channel_list.into_iter().map(|(channel, name)| async move {
        if let Err(err) = channel.join().await {
            log::error!("error sending to metric server {name}: {err}");
        }
    }))
    .await;

    Ok(())
}

/// Get the metric server connections from a config
pub fn get_metric_server_connections(
    metric_config: proxmox_section_config::SectionConfigData,
) -> Result<Vec<(proxmox_metrics::Metrics, String)>, Error> {
    let mut res = Vec::new();

    for config in
        metric_config.convert_to_typed_array::<pbs_api_types::InfluxDbUdp>("influxdb-udp")?
    {
        if !config.enable {
            continue;
        }
        let future = proxmox_metrics::influxdb_udp(&config.host, config.mtu);
        res.push((future, config.name));
    }

    for config in
        metric_config.convert_to_typed_array::<pbs_api_types::InfluxDbHttp>("influxdb-http")?
    {
        if !config.enable {
            continue;
        }
        let future = proxmox_metrics::influxdb_http(
            &config.url,
            config.organization.as_deref().unwrap_or("proxmox"),
            config.bucket.as_deref().unwrap_or("proxmox"),
            config.token.as_deref(),
            config.verify_tls.unwrap_or(true),
            config.max_body_size.unwrap_or(25_000_000),
        )?;
        res.push((future, config.name));
    }
    Ok(res)
}

struct HostStats {
    proc: Option<ProcFsStat>,
    meminfo: Option<ProcFsMemInfo>,
    net: Option<Vec<ProcFsNetDev>>,
    load: Option<Loadavg>,
}

struct DiskStat {
    name: String,
    usage: Option<FileSystemInformation>,
    dev: Option<BlockDevStat>,
}

impl DiskStat {
    fn to_value(&self) -> Value {
        let mut value = json!({});
        if let Some(usage) = &self.usage {
            value["total"] = Value::from(usage.total);
            value["used"] = Value::from(usage.used);
            value["avail"] = Value::from(usage.available);
        }

        if let Some(dev) = &self.dev {
            value["read_ios"] = Value::from(dev.read_ios);
            value["read_bytes"] = Value::from(dev.read_sectors * 512);
            value["write_ios"] = Value::from(dev.write_ios);
            value["write_bytes"] = Value::from(dev.write_sectors * 512);
            value["io_ticks"] = Value::from(dev.io_ticks / 1000);
        }
        value
    }
}

fn collect_host_stats_sync() -> HostStats {
    use proxmox_sys::linux::procfs::{
        read_loadavg, read_meminfo, read_proc_net_dev, read_proc_stat,
    };

    let proc = match read_proc_stat() {
        Ok(stat) => Some(stat),
        Err(err) => {
            eprintln!("read_proc_stat failed - {err}");
            None
        }
    };

    let meminfo = match read_meminfo() {
        Ok(stat) => Some(stat),
        Err(err) => {
            eprintln!("read_meminfo failed - {err}");
            None
        }
    };

    let net = match read_proc_net_dev() {
        Ok(netdev) => Some(netdev),
        Err(err) => {
            eprintln!("read_prox_net_dev failed - {err}");
            None
        }
    };

    let load = match read_loadavg() {
        Ok(loadavg) => Some(loadavg),
        Err(err) => {
            eprintln!("read_loadavg failed - {err}");
            None
        }
    };

    HostStats {
        proc,
        meminfo,
        net,
        load,
    }
}

fn collect_disk_stats_sync() -> (DiskStat, Vec<DiskStat>) {
    let disk_manager = DiskManage::new();

    let root = gather_disk_stats(disk_manager.clone(), Path::new("/"), "host");

    let mut datastores = Vec::new();
    match pbs_config::datastore::config() {
        Ok((config, _)) => {
            let datastore_list: Vec<DataStoreConfig> = config
                .convert_to_typed_array("datastore")
                .unwrap_or_default();

            for config in datastore_list {
                if config
                    .get_maintenance_mode()
                    .map_or(false, |mode| mode.check(Some(Operation::Read)).is_err())
                {
                    continue;
                }
                let path = std::path::Path::new(&config.path);
                datastores.push(gather_disk_stats(disk_manager.clone(), path, &config.name));
            }
        }
        Err(err) => {
            eprintln!("read datastore config failed - {err}");
        }
    }

    (root, datastores)
}

fn rrd_update_host_stats_sync(host: &HostStats, hostdisk: &DiskStat, datastores: &[DiskStat]) {
    if let Some(stat) = &host.proc {
        rrd_update_gauge("host/cpu", stat.cpu);
        rrd_update_gauge("host/iowait", stat.iowait_percent);
    }

    if let Some(meminfo) = &host.meminfo {
        rrd_update_gauge("host/memtotal", meminfo.memtotal as f64);
        rrd_update_gauge("host/memused", meminfo.memused as f64);
        rrd_update_gauge("host/swaptotal", meminfo.swaptotal as f64);
        rrd_update_gauge("host/swapused", meminfo.swapused as f64);
    }

    if let Some(netdev) = &host.net {
        use pbs_config::network::is_physical_nic;
        let mut netin = 0;
        let mut netout = 0;
        for item in netdev {
            if !is_physical_nic(&item.device) {
                continue;
            }
            netin += item.receive;
            netout += item.send;
        }
        rrd_update_derive("host/netin", netin as f64);
        rrd_update_derive("host/netout", netout as f64);
    }

    if let Some(loadavg) = &host.load {
        rrd_update_gauge("host/loadavg", loadavg.0);
    }

    rrd_update_disk_stat(hostdisk, "host");

    for stat in datastores {
        let rrd_prefix = format!("datastore/{}", stat.name);
        rrd_update_disk_stat(stat, &rrd_prefix);
    }
}

fn rrd_update_disk_stat(disk: &DiskStat, rrd_prefix: &str) {
    if let Some(status) = &disk.usage {
        let rrd_key = format!("{}/total", rrd_prefix);
        rrd_update_gauge(&rrd_key, status.total as f64);
        let rrd_key = format!("{}/used", rrd_prefix);
        rrd_update_gauge(&rrd_key, status.used as f64);
        let rrd_key = format!("{}/available", rrd_prefix);
        rrd_update_gauge(&rrd_key, status.available as f64);
    }

    if let Some(stat) = &disk.dev {
        let rrd_key = format!("{}/read_ios", rrd_prefix);
        rrd_update_derive(&rrd_key, stat.read_ios as f64);
        let rrd_key = format!("{}/read_bytes", rrd_prefix);
        rrd_update_derive(&rrd_key, (stat.read_sectors * 512) as f64);

        let rrd_key = format!("{}/write_ios", rrd_prefix);
        rrd_update_derive(&rrd_key, stat.write_ios as f64);
        let rrd_key = format!("{}/write_bytes", rrd_prefix);
        rrd_update_derive(&rrd_key, (stat.write_sectors * 512) as f64);

        let rrd_key = format!("{}/io_ticks", rrd_prefix);
        rrd_update_derive(&rrd_key, (stat.io_ticks as f64) / 1000.0);
    }
}

fn check_schedule(worker_type: &str, event_str: &str, id: &str) -> bool {
    let event: CalendarEvent = match event_str.parse() {
        Ok(event) => event,
        Err(err) => {
            eprintln!("unable to parse schedule '{event_str}' - {err}");
            return false;
        }
    };

    let last = match jobstate::last_run_time(worker_type, id) {
        Ok(time) => time,
        Err(err) => {
            eprintln!("could not get last run time of {worker_type} {id}: {err}");
            return false;
        }
    };

    let next = match event.compute_next_event(last) {
        Ok(Some(next)) => next,
        Ok(None) => return false,
        Err(err) => {
            eprintln!("compute_next_event for '{event_str}' failed - {err}");
            return false;
        }
    };

    let now = proxmox_time::epoch_i64();
    next <= now
}

fn gather_disk_stats(disk_manager: Arc<DiskManage>, path: &Path, name: &str) -> DiskStat {
    let usage = match proxmox_sys::fs::fs_info(path) {
        Ok(status) => Some(status),
        Err(err) => {
            eprintln!("read fs info on {path:?} failed - {err}");
            None
        }
    };

    let dev = match disk_manager.find_mounted_device(path) {
        Ok(None) => None,
        Ok(Some((fs_type, device, source))) => {
            let mut device_stat = None;
            match (fs_type.as_str(), source) {
                ("zfs", Some(source)) => match source.into_string() {
                    Ok(dataset) => match zfs_dataset_stats(&dataset) {
                        Ok(stat) => device_stat = Some(stat),
                        Err(err) => eprintln!("zfs_dataset_stats({dataset:?}) failed - {err}"),
                    },
                    Err(source) => {
                        eprintln!("zfs_pool_stats({source:?}) failed - invalid characters")
                    }
                },
                _ => {
                    if let Ok(disk) = disk_manager.clone().disk_by_dev_num(device.into_dev_t()) {
                        match disk.read_stat() {
                            Ok(stat) => device_stat = stat,
                            Err(err) => eprintln!("disk.read_stat {path:?} failed - {err}"),
                        }
                    }
                }
            }
            device_stat
        }
        Err(err) => {
            eprintln!("find_mounted_device failed - {err}");
            None
        }
    };

    DiskStat {
        name: name.to_string(),
        usage,
        dev,
    }
}

// Rate Limiter lookup
async fn run_traffic_control_updater() {
    loop {
        let delay_target = Instant::now() + Duration::from_secs(1);

        {
            let mut cache = TRAFFIC_CONTROL_CACHE.lock().unwrap();
            cache.compute_current_rates();
        }

        tokio::time::sleep_until(tokio::time::Instant::from_std(delay_target)).await;
    }
}

fn lookup_rate_limiter(
    peer: std::net::SocketAddr,
) -> (Option<SharedRateLimit>, Option<SharedRateLimit>) {
    let mut cache = TRAFFIC_CONTROL_CACHE.lock().unwrap();

    let now = proxmox_time::epoch_i64();

    cache.reload(now);

    let (_rule_name, read_limiter, write_limiter) = cache.lookup_rate_limiter(peer, now);

    (read_limiter, write_limiter)
}
