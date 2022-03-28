use std::future::Future;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use anyhow::{bail, format_err, Error};
use futures::*;
use http::request::Parts;
use http::Response;
use hyper::header;
use hyper::{Body, StatusCode};
use url::form_urlencoded;

use http::{HeaderMap, Method};
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use serde_json::{json, Value};
use tokio_stream::wrappers::ReceiverStream;

use proxmox_http::client::{RateLimitedStream, ShareableRateLimit};
use proxmox_lang::try_block;
use proxmox_router::{RpcEnvironment, RpcEnvironmentType, UserInformation};
use proxmox_sys::fs::CreateOptions;
use proxmox_sys::linux::socket::set_tcp_keepalive;
use proxmox_sys::logrotate::LogRotate;
use proxmox_sys::{task_log, task_warn};

use pbs_datastore::DataStore;

use proxmox_rest_server::{
    cleanup_old_tasks, cookie_from_header, rotate_task_log_archive, ApiConfig, AuthError,
    RestEnvironment, RestServer, ServerAdapter, WorkerTask,
};

use proxmox_backup::rrd_cache::{
    initialize_rrd_cache, rrd_sync_journal, rrd_update_derive, rrd_update_gauge,
};
use proxmox_backup::{
    server::{
        auth::check_pbs_auth,
        jobstate::{self, Job},
    },
    traffic_control_cache::TRAFFIC_CONTROL_CACHE,
};

use pbs_buildcfg::configdir;
use proxmox_time::CalendarEvent;

use pbs_api_types::{
    Authid, DataStoreConfig, PruneOptions, SyncJobConfig, TapeBackupJobConfig,
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
            "proxy not running as backup user or group (got uid {} gid {})",
            running_uid,
            running_gid
        );
    }

    proxmox_async::runtime::main(run())
}

struct ProxmoxBackupProxyAdapter;

impl ServerAdapter for ProxmoxBackupProxyAdapter {
    fn get_index(
        &self,
        env: RestEnvironment,
        parts: Parts,
    ) -> Pin<Box<dyn Future<Output = Response<Body>> + Send>> {
        Box::pin(get_index_future(env, parts))
    }

    fn check_auth<'a>(
        &'a self,
        headers: &'a HeaderMap,
        method: &'a Method,
    ) -> Pin<Box<dyn Future<Output = Result<(String, Box<dyn UserInformation + Sync + Send>), AuthError>> + Send + 'a>> {
        Box::pin(async move {
            check_pbs_auth(headers, method).await
        })
    }
}

/// check for a cookie with the user-preferred language, fallback to the config one if not set or
/// not existing
fn get_language(headers: &http::HeaderMap) -> String {
    let exists = |l: &str| Path::new(&format!("/usr/share/pbs-i18n/pbs-lang-{}.js", l)).exists();

    match cookie_from_header(headers, "PBSLangCookie") {
        Some(cookie_lang) if exists(&cookie_lang) => cookie_lang,
        _ => match proxmox_backup::config::node::config().map(|(cfg, _)| cfg.default_lang) {
            Ok(Some(default_lang)) if exists(&default_lang) => default_lang,
            _ => String::from(""),
        },
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

    let data = json!({
        "NodeName": nodename,
        "UserName": user,
        "CSRFPreventionToken": csrf_token,
        "language": get_language(&parts.headers),
        "debug": debug,
    });

    let (ct, index) = match api.render_template(template_file, &data) {
        Ok(index) => ("text/html", index),
        Err(err) => ("text/plain", format!("Error rendering template: {}", err)),
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
        if debug { log::LevelFilter::Debug } else { log::LevelFilter::Info },
        Some("proxmox-backup-proxy"),
    ) {
        bail!("unable to inititialize syslog - {}", err);
    }

    let _ = public_auth_key(); // load with lazy_static
    let _ = csrf_secret(); // load with lazy_static

    let rrd_cache = initialize_rrd_cache()?;
    rrd_cache.apply_journal()?;

    let mut config = ApiConfig::new(
        pbs_buildcfg::JS_DIR,
        &proxmox_backup::api2::ROUTER,
        RpcEnvironmentType::PUBLIC,
        ProxmoxBackupProxyAdapter,
    )?;

    config.add_alias("novnc", "/usr/share/novnc-pve");
    config.add_alias("extjs", "/usr/share/javascript/extjs");
    config.add_alias("qrcodejs", "/usr/share/javascript/qrcodejs");
    config.add_alias("fontawesome", "/usr/share/fonts-font-awesome");
    config.add_alias("xtermjs", "/usr/share/pve-xtermjs");
    config.add_alias("locale", "/usr/share/pbs-i18n");
    config.add_alias(
        "widgettoolkit",
        "/usr/share/javascript/proxmox-widget-toolkit",
    );
    config.add_alias("docs", "/usr/share/doc/proxmox-backup/html");

    let mut indexpath = PathBuf::from(pbs_buildcfg::JS_DIR);
    indexpath.push("index.hbs");
    config.register_template("index", &indexpath)?;
    config.register_template("console", "/usr/share/pve-xtermjs/index.html.hbs")?;

    let backup_user = pbs_config::backup_user()?;
    let mut commando_sock = proxmox_rest_server::CommandSocket::new(
        proxmox_rest_server::our_ctrl_sock(),
        backup_user.gid,
    );

    let dir_opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);
    let file_opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);

    config.enable_access_log(
        pbs_buildcfg::API_ACCESS_LOG_FN,
        Some(dir_opts.clone()),
        Some(file_opts.clone()),
        &mut commando_sock,
    )?;

    config.enable_auth_log(
        pbs_buildcfg::API_AUTH_LOG_FN,
        Some(dir_opts.clone()),
        Some(file_opts.clone()),
        &mut commando_sock,
    )?;

    let rest_server = RestServer::new(config);
    proxmox_rest_server::init_worker_tasks(
        pbs_buildcfg::PROXMOX_BACKUP_LOG_DIR_M!().into(),
        file_opts.clone(),
    )?;

    //openssl req -x509 -newkey rsa:4096 -keyout /etc/proxmox-backup/proxy.key -out /etc/proxmox-backup/proxy.pem -nodes

    // we build the initial acceptor here as we cannot start if this fails
    let acceptor = make_tls_acceptor()?;
    let acceptor = Arc::new(Mutex::new(acceptor));

    // to renew the acceptor we just add a command-socket handler
    commando_sock.register_command("reload-certificate".to_string(), {
        let acceptor = Arc::clone(&acceptor);
        move |_value| -> Result<_, Error> {
            log::info!("reloading certificate");
            match make_tls_acceptor() {
                Err(err) => log::error!("error reloading certificate: {}", err),
                Ok(new_acceptor) => {
                    let mut guard = acceptor.lock().unwrap();
                    *guard = new_acceptor;
                }
            }
            Ok(Value::Null)
        }
    })?;

    // to remove references for not configured datastores
    commando_sock.register_command("datastore-removed".to_string(), |_value| {
        if let Err(err) = DataStore::remove_unused_datastores() {
            log::error!("could not refresh datastores: {}", err);
        }
        Ok(Value::Null)
    })?;

    let server = daemon::create_daemon(([0, 0, 0, 0, 0, 0, 0, 0], 8007).into(), move |listener| {
        let connections = accept_connections(listener, acceptor, debug);
        let connections = hyper::server::accept::from_stream(ReceiverStream::new(connections));

        Ok(async {
            daemon::systemd_notify(daemon::SystemdNotify::Ready)?;

            hyper::Server::builder(connections)
                .serve(rest_server)
                .with_graceful_shutdown(proxmox_rest_server::shutdown_future())
                .map_err(Error::from)
                .await
        })
    });

    proxmox_rest_server::write_pid(pbs_buildcfg::PROXMOX_BACKUP_PROXY_PID_FN)?;

    let init_result: Result<(), Error> = try_block!({
        proxmox_rest_server::register_task_control_commands(&mut commando_sock)?;
        commando_sock.spawn()?;
        proxmox_rest_server::catch_shutdown_signal()?;
        proxmox_rest_server::catch_reload_signal()?;
        Ok(())
    });

    if let Err(err) = init_result {
        bail!("unable to start daemon - {}", err);
    }

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

    let mut acceptor = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls()).unwrap();
    if let Some(ciphers) = ciphers_tls_1_3.as_deref() {
        acceptor.set_ciphersuites(ciphers)?;
    }
    if let Some(ciphers) = ciphers_tls_1_2.as_deref() {
        acceptor.set_cipher_list(ciphers)?;
    }
    acceptor
        .set_private_key_file(key_path, SslFiletype::PEM)
        .map_err(|err| format_err!("unable to read proxy key {} - {}", key_path, err))?;
    acceptor
        .set_certificate_chain_file(cert_path)
        .map_err(|err| format_err!("unable to read proxy cert {} - {}", cert_path, err))?;
    acceptor.set_options(openssl::ssl::SslOptions::NO_RENEGOTIATION);
    acceptor.check_private_key().unwrap();

    Ok(acceptor.build())
}

type ClientStreamResult = Result<
    std::pin::Pin<Box<tokio_openssl::SslStream<RateLimitedStream<tokio::net::TcpStream>>>>,
    Error,
>;
const MAX_PENDING_ACCEPTS: usize = 1024;

fn accept_connections(
    listener: tokio::net::TcpListener,
    acceptor: Arc<Mutex<openssl::ssl::SslAcceptor>>,
    debug: bool,
) -> tokio::sync::mpsc::Receiver<ClientStreamResult> {
    let (sender, receiver) = tokio::sync::mpsc::channel(MAX_PENDING_ACCEPTS);

    tokio::spawn(accept_connection(listener, acceptor, debug, sender));

    receiver
}

async fn accept_connection(
    listener: tokio::net::TcpListener,
    acceptor: Arc<Mutex<openssl::ssl::SslAcceptor>>,
    debug: bool,
    sender: tokio::sync::mpsc::Sender<ClientStreamResult>,
) {
    let accept_counter = Arc::new(());

    loop {
        let (sock, peer) = match listener.accept().await {
            Ok(conn) => conn,
            Err(err) => {
                eprintln!("error accepting tcp connection: {}", err);
                continue;
            }
        };

        sock.set_nodelay(true).unwrap();
        let _ = set_tcp_keepalive(sock.as_raw_fd(), PROXMOX_BACKUP_TCP_KEEPALIVE_TIME);

        let sock =
            RateLimitedStream::with_limiter_update_cb(sock, move || lookup_rate_limiter(peer));

        let ssl = {
            // limit acceptor_guard scope
            // Acceptor can be reloaded using the command socket "reload-certificate" command
            let acceptor_guard = acceptor.lock().unwrap();

            match openssl::ssl::Ssl::new(acceptor_guard.context()) {
                Ok(ssl) => ssl,
                Err(err) => {
                    eprintln!(
                        "failed to create Ssl object from Acceptor context - {}",
                        err
                    );
                    continue;
                }
            }
        };

        let stream = match tokio_openssl::SslStream::new(ssl, sock) {
            Ok(stream) => stream,
            Err(err) => {
                eprintln!(
                    "failed to create SslStream using ssl and connection socket - {}",
                    err
                );
                continue;
            }
        };

        let mut stream = Box::pin(stream);
        let sender = sender.clone();

        if Arc::strong_count(&accept_counter) > MAX_PENDING_ACCEPTS {
            eprintln!("connection rejected - to many open connections");
            continue;
        }

        let accept_counter = Arc::clone(&accept_counter);
        tokio::spawn(async move {
            let accept_future =
                tokio::time::timeout(Duration::new(10, 0), stream.as_mut().accept());

            let result = accept_future.await;

            match result {
                Ok(Ok(())) => {
                    if sender.send(Ok(stream)).await.is_err() && debug {
                        eprintln!("detect closed connection channel");
                    }
                }
                Ok(Err(err)) => {
                    if debug {
                        eprintln!("https handshake failed - {}", err);
                    }
                }
                Err(_) => {
                    if debug {
                        eprintln!("https handshake timeout");
                    }
                }
            }

            drop(accept_counter); // decrease reference count
        });
    }
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

fn next_minute() -> Result<Instant, Error> {
    let now = SystemTime::now();
    let epoch_now = now.duration_since(UNIX_EPOCH)?;
    let epoch_next = Duration::from_secs((epoch_now.as_secs() / 60 + 1) * 60);
    Ok(Instant::now() + epoch_next - epoch_now)
}

async fn run_task_scheduler() {
    let mut count: usize = 0;

    loop {
        count += 1;

        let delay_target = match next_minute() {
            // try to run very minute
            Ok(d) => d,
            Err(err) => {
                eprintln!("task scheduler: compute next minute failed - {}", err);
                tokio::time::sleep_until(tokio::time::Instant::from_std(
                    Instant::now() + Duration::from_secs(60),
                ))
                .await;
                continue;
            }
        };

        if count > 2 {
            // wait 1..2 minutes before starting
            match schedule_tasks().catch_unwind().await {
                Err(panic) => match panic.downcast::<&str>() {
                    Ok(msg) => {
                        eprintln!("task scheduler panic: {}", msg);
                    }
                    Err(_) => {
                        eprintln!("task scheduler panic - unknown type");
                    }
                },
                Ok(Err(err)) => {
                    eprintln!("task scheduler failed - {:?}", err);
                }
                Ok(Ok(_)) => {}
            }
        }

        tokio::time::sleep_until(tokio::time::Instant::from_std(delay_target)).await;
    }
}

async fn schedule_tasks() -> Result<(), Error> {
    schedule_datastore_garbage_collection().await;
    schedule_datastore_prune().await;
    schedule_datastore_sync_jobs().await;
    schedule_datastore_verify_jobs().await;
    schedule_tape_backup_jobs().await;
    schedule_task_log_rotate().await;

    Ok(())
}

async fn schedule_datastore_garbage_collection() {
    let config = match pbs_config::datastore::config() {
        Err(err) => {
            eprintln!("unable to read datastore config - {}", err);
            return;
        }
        Ok((config, _digest)) => config,
    };

    for (store, (_, store_config)) in config.sections {
        let datastore = match DataStore::lookup_datastore(&store) {
            Ok(datastore) => datastore,
            Err(err) => {
                eprintln!("lookup_datastore failed - {}", err);
                continue;
            }
        };

        let store_config: DataStoreConfig = match serde_json::from_value(store_config) {
            Ok(c) => c,
            Err(err) => {
                eprintln!("datastore config from_value failed - {}", err);
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
                eprintln!("unable to parse schedule '{}' - {}", event_str, err);
                continue;
            }
        };

        if datastore.garbage_collection_running() {
            continue;
        }

        let worker_type = "garbage_collection";

        let last = match jobstate::last_run_time(worker_type, &store) {
            Ok(time) => time,
            Err(err) => {
                eprintln!(
                    "could not get last run time of {} {}: {}",
                    worker_type, store, err
                );
                continue;
            }
        };

        let next = match event.compute_next_event(last) {
            Ok(Some(next)) => next,
            Ok(None) => continue,
            Err(err) => {
                eprintln!("compute_next_event for '{}' failed - {}", event_str, err);
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

        let auth_id = Authid::root_auth_id();

        if let Err(err) = crate::server::do_garbage_collection_job(
            job,
            datastore,
            auth_id,
            Some(event_str),
            false,
        ) {
            eprintln!(
                "unable to start garbage collection job on datastore {} - {}",
                store, err
            );
        }
    }
}

async fn schedule_datastore_prune() {
    let config = match pbs_config::datastore::config() {
        Err(err) => {
            eprintln!("unable to read datastore config - {}", err);
            return;
        }
        Ok((config, _digest)) => config,
    };

    for (store, (_, store_config)) in config.sections {
        let store_config: DataStoreConfig = match serde_json::from_value(store_config) {
            Ok(c) => c,
            Err(err) => {
                eprintln!("datastore '{}' config from_value failed - {}", store, err);
                continue;
            }
        };

        let event_str = match store_config.prune_schedule {
            Some(event_str) => event_str,
            None => continue,
        };

        let prune_options = PruneOptions {
            keep_last: store_config.keep_last,
            keep_hourly: store_config.keep_hourly,
            keep_daily: store_config.keep_daily,
            keep_weekly: store_config.keep_weekly,
            keep_monthly: store_config.keep_monthly,
            keep_yearly: store_config.keep_yearly,
        };

        if !pbs_datastore::prune::keeps_something(&prune_options) {
            // no prune settings - keep all
            continue;
        }

        let worker_type = "prune";
        if check_schedule(worker_type, &event_str, &store) {
            let job = match Job::new(worker_type, &store) {
                Ok(job) => job,
                Err(_) => continue, // could not get lock
            };

            let auth_id = Authid::root_auth_id().clone();
            if let Err(err) =
                do_prune_job(job, prune_options, store.clone(), &auth_id, Some(event_str))
            {
                eprintln!("unable to start datastore prune job {} - {}", &store, err);
            }
        };
    }
}

async fn schedule_datastore_sync_jobs() {
    let config = match pbs_config::sync::config() {
        Err(err) => {
            eprintln!("unable to read sync job config - {}", err);
            return;
        }
        Ok((config, _digest)) => config,
    };

    for (job_id, (_, job_config)) in config.sections {
        let job_config: SyncJobConfig = match serde_json::from_value(job_config) {
            Ok(c) => c,
            Err(err) => {
                eprintln!("sync job config from_value failed - {}", err);
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
                eprintln!("unable to start datastore sync job {} - {}", &job_id, err);
            }
        };
    }
}

async fn schedule_datastore_verify_jobs() {
    let config = match pbs_config::verify::config() {
        Err(err) => {
            eprintln!("unable to read verification job config - {}", err);
            return;
        }
        Ok((config, _digest)) => config,
    };
    for (job_id, (_, job_config)) in config.sections {
        let job_config: VerificationJobConfig = match serde_json::from_value(job_config) {
            Ok(c) => c,
            Err(err) => {
                eprintln!("verification job config from_value failed - {}", err);
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
                eprintln!(
                    "unable to start datastore verification job {} - {}",
                    &job_id, err
                );
            }
        };
    }
}

async fn schedule_tape_backup_jobs() {
    let config = match pbs_config::tape_job::config() {
        Err(err) => {
            eprintln!("unable to read tape job config - {}", err);
            return;
        }
        Ok((config, _digest)) => config,
    };
    for (job_id, (_, job_config)) in config.sections {
        let job_config: TapeBackupJobConfig = match serde_json::from_value(job_config) {
            Ok(c) => c,
            Err(err) => {
                eprintln!("tape backup job config from_value failed - {}", err);
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
                eprintln!("unable to start tape backup job {} - {}", &job_id, err);
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
            Ok(state) => match state {
                jobstate::JobState::Created { .. } => {}
                _ => return,
            },
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
                        task_warn!(worker, "could not completely cleanup old tasks: {}", err);
                    }
                }

                Ok(())
            });

            let status = worker.create_state(&result);

            if let Err(err) = job.finish(status) {
                eprintln!("could not finish job state for {}: {}", worker_type, err);
            }

            result
        },
    ) {
        eprintln!("unable to start task log rotation: {}", err);
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
            "reopen commands failed, proxy: {}; api: {}",
            e1,
            e2
        )),
        (Err(e1), Ok(_)) => Err(format_err!("reopen commands failed, proxy: {}", e1)),
        (Ok(_), Err(e2)) => Err(format_err!("reopen commands failed, api: {}", e2)),
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
            "reopen commands failed, proxy: {}; api: {}",
            e1,
            e2
        )),
        (Err(e1), Ok(_)) => Err(format_err!("reopen commands failed, proxy: {}", e1)),
        (Ok(_), Err(e2)) => Err(format_err!("reopen commands failed, api: {}", e2)),
        _ => Ok(()),
    }
}

async fn run_stat_generator() {
    loop {
        let delay_target = Instant::now() + Duration::from_secs(10);

        generate_host_stats().await;

        rrd_sync_journal();

        tokio::time::sleep_until(tokio::time::Instant::from_std(delay_target)).await;
    }
}

async fn generate_host_stats() {
    match tokio::task::spawn_blocking(generate_host_stats_sync).await {
        Ok(()) => (),
        Err(err) => log::error!("generate_host_stats paniced: {}", err),
    }
}

fn generate_host_stats_sync() {
    use proxmox_sys::linux::procfs::{
        read_loadavg, read_meminfo, read_proc_net_dev, read_proc_stat,
    };

    match read_proc_stat() {
        Ok(stat) => {
            rrd_update_gauge("host/cpu", stat.cpu);
            rrd_update_gauge("host/iowait", stat.iowait_percent);
        }
        Err(err) => {
            eprintln!("read_proc_stat failed - {}", err);
        }
    }

    match read_meminfo() {
        Ok(meminfo) => {
            rrd_update_gauge("host/memtotal", meminfo.memtotal as f64);
            rrd_update_gauge("host/memused", meminfo.memused as f64);
            rrd_update_gauge("host/swaptotal", meminfo.swaptotal as f64);
            rrd_update_gauge("host/swapused", meminfo.swapused as f64);
        }
        Err(err) => {
            eprintln!("read_meminfo failed - {}", err);
        }
    }

    match read_proc_net_dev() {
        Ok(netdev) => {
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
        Err(err) => {
            eprintln!("read_prox_net_dev failed - {}", err);
        }
    }

    match read_loadavg() {
        Ok(loadavg) => {
            rrd_update_gauge("host/loadavg", loadavg.0 as f64);
        }
        Err(err) => {
            eprintln!("read_loadavg failed - {}", err);
        }
    }

    let disk_manager = DiskManage::new();

    gather_disk_stats(disk_manager.clone(), Path::new("/"), "host");

    match pbs_config::datastore::config() {
        Ok((config, _)) => {
            let datastore_list: Vec<DataStoreConfig> = config
                .convert_to_typed_array("datastore")
                .unwrap_or_default();

            for config in datastore_list {
                let rrd_prefix = format!("datastore/{}", config.name);
                let path = std::path::Path::new(&config.path);
                gather_disk_stats(disk_manager.clone(), path, &rrd_prefix);
            }
        }
        Err(err) => {
            eprintln!("read datastore config failed - {}", err);
        }
    }
}

fn check_schedule(worker_type: &str, event_str: &str, id: &str) -> bool {
    let event: CalendarEvent = match event_str.parse() {
        Ok(event) => event,
        Err(err) => {
            eprintln!("unable to parse schedule '{}' - {}", event_str, err);
            return false;
        }
    };

    let last = match jobstate::last_run_time(worker_type, id) {
        Ok(time) => time,
        Err(err) => {
            eprintln!(
                "could not get last run time of {} {}: {}",
                worker_type, id, err
            );
            return false;
        }
    };

    let next = match event.compute_next_event(last) {
        Ok(Some(next)) => next,
        Ok(None) => return false,
        Err(err) => {
            eprintln!("compute_next_event for '{}' failed - {}", event_str, err);
            return false;
        }
    };

    let now = proxmox_time::epoch_i64();
    next <= now
}

fn gather_disk_stats(disk_manager: Arc<DiskManage>, path: &Path, rrd_prefix: &str) {
    match proxmox_backup::tools::disks::disk_usage(path) {
        Ok(status) => {
            let rrd_key = format!("{}/total", rrd_prefix);
            rrd_update_gauge(&rrd_key, status.total as f64);
            let rrd_key = format!("{}/used", rrd_prefix);
            rrd_update_gauge(&rrd_key, status.used as f64);
        }
        Err(err) => {
            eprintln!("read disk_usage on {:?} failed - {}", path, err);
        }
    }

    match disk_manager.find_mounted_device(path) {
        Ok(None) => {}
        Ok(Some((fs_type, device, source))) => {
            let mut device_stat = None;
            match (fs_type.as_str(), source) {
                ("zfs", Some(source)) => match source.into_string() {
                    Ok(dataset) => match zfs_dataset_stats(&dataset) {
                        Ok(stat) => device_stat = Some(stat),
                        Err(err) => eprintln!("zfs_dataset_stats({:?}) failed - {}", dataset, err),
                    },
                    Err(source) => {
                        eprintln!("zfs_pool_stats({:?}) failed - invalid characters", source)
                    }
                },
                _ => {
                    if let Ok(disk) = disk_manager.clone().disk_by_dev_num(device.into_dev_t()) {
                        match disk.read_stat() {
                            Ok(stat) => device_stat = stat,
                            Err(err) => eprintln!("disk.read_stat {:?} failed - {}", path, err),
                        }
                    }
                }
            }
            if let Some(stat) = device_stat {
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
        Err(err) => {
            eprintln!("find_mounted_device failed - {}", err);
        }
    }
}

// Rate Limiter lookup

// Test WITH
// proxmox-backup-client restore vm/201/2021-10-22T09:55:56Z drive-scsi0.img img1.img --repository localhost:store2

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
) -> (
    Option<Arc<dyn ShareableRateLimit>>,
    Option<Arc<dyn ShareableRateLimit>>,
) {
    let mut cache = TRAFFIC_CONTROL_CACHE.lock().unwrap();

    let now = proxmox_time::epoch_i64();

    cache.reload(now);

    let (_rule_name, read_limiter, write_limiter) = cache.lookup_rate_limiter(peer, now);

    (read_limiter, write_limiter)
}
