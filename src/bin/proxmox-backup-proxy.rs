use std::sync::{Mutex, Arc};
use std::path::{Path, PathBuf};
use std::os::unix::io::AsRawFd;

use anyhow::{bail, format_err, Error};
use futures::*;

use openssl::ssl::{SslMethod, SslAcceptor, SslFiletype};
use tokio_stream::wrappers::ReceiverStream;
use serde_json::Value;

use proxmox::try_block;
use proxmox::api::RpcEnvironmentType;
use proxmox::sys::linux::socket::set_tcp_keepalive;

use proxmox_backup::{
    backup::DataStore,
    server::{
        auth::default_api_auth,
        WorkerTask,
        ApiConfig,
        rest::*,
        jobstate::{
            self,
            Job,
        },
        rotate_task_log_archive,
    },
};

use pbs_buildcfg::configdir;
use pbs_systemd::time::{compute_next_event, parse_calendar_event};
use pbs_tools::logrotate::LogRotate;

use proxmox_backup::api2::types::Authid;
use proxmox_backup::server;
use proxmox_backup::auth_helpers::*;
use proxmox_backup::tools::{
    PROXMOX_BACKUP_TCP_KEEPALIVE_TIME,
    daemon,
    disks::{
        DiskManage,
        zfs_pool_stats,
        get_pool_from_dataset,
    },
};

use proxmox_backup::api2::pull::do_sync_job;
use proxmox_backup::api2::tape::backup::do_tape_backup_job;
use proxmox_backup::server::do_verification_job;
use proxmox_backup::server::do_prune_job;

fn main() -> Result<(), Error> {
    proxmox_backup::tools::setup_safe_path_env();

    let backup_uid = pbs_config::backup_user()?.uid;
    let backup_gid = pbs_config::backup_group()?.gid;
    let running_uid = nix::unistd::Uid::effective();
    let running_gid = nix::unistd::Gid::effective();

    if running_uid != backup_uid || running_gid != backup_gid {
        bail!("proxy not running as backup user or group (got uid {} gid {})", running_uid, running_gid);
    }

    pbs_runtime::main(run())
}

async fn run() -> Result<(), Error> {
    if let Err(err) = syslog::init(
        syslog::Facility::LOG_DAEMON,
        log::LevelFilter::Info,
        Some("proxmox-backup-proxy")) {
        bail!("unable to inititialize syslog - {}", err);
    }

    // Note: To debug early connection error use
    // PROXMOX_DEBUG=1 ./target/release/proxmox-backup-proxy
    let debug = std::env::var("PROXMOX_DEBUG").is_ok();

    let _ = public_auth_key(); // load with lazy_static
    let _ = csrf_secret(); // load with lazy_static

    let mut config = ApiConfig::new(
        pbs_buildcfg::JS_DIR,
        &proxmox_backup::api2::ROUTER,
        RpcEnvironmentType::PUBLIC,
        default_api_auth(),
    )?;

    config.add_alias("novnc", "/usr/share/novnc-pve");
    config.add_alias("extjs", "/usr/share/javascript/extjs");
    config.add_alias("qrcodejs", "/usr/share/javascript/qrcodejs");
    config.add_alias("fontawesome", "/usr/share/fonts-font-awesome");
    config.add_alias("xtermjs", "/usr/share/pve-xtermjs");
    config.add_alias("locale", "/usr/share/pbs-i18n");
    config.add_alias("widgettoolkit", "/usr/share/javascript/proxmox-widget-toolkit");
    config.add_alias("docs", "/usr/share/doc/proxmox-backup/html");

    let mut indexpath = PathBuf::from(pbs_buildcfg::JS_DIR);
    indexpath.push("index.hbs");
    config.register_template("index", &indexpath)?;
    config.register_template("console", "/usr/share/pve-xtermjs/index.html.hbs")?;

    let mut commando_sock = server::CommandoSocket::new(server::our_ctrl_sock());

    config.enable_file_log(pbs_buildcfg::API_ACCESS_LOG_FN, &mut commando_sock)?;

    let rest_server = RestServer::new(config);

    //openssl req -x509 -newkey rsa:4096 -keyout /etc/proxmox-backup/proxy.key -out /etc/proxmox-backup/proxy.pem -nodes

    // we build the initial acceptor here as we cannot start if this fails
    let acceptor = make_tls_acceptor()?;
    let acceptor = Arc::new(Mutex::new(acceptor));

    // to renew the acceptor we just add a command-socket handler
    commando_sock.register_command(
        "reload-certificate".to_string(),
        {
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
        },
    )?;

    // to remove references for not configured datastores
    commando_sock.register_command(
        "datastore-removed".to_string(),
        |_value| {
            if let Err(err) = proxmox_backup::backup::DataStore::remove_unused_datastores() {
                log::error!("could not refresh datastores: {}", err);
            }
            Ok(Value::Null)
        }
    )?;

    let server = daemon::create_daemon(
        ([0,0,0,0,0,0,0,0], 8007).into(),
        move |listener, ready| {

            let connections = accept_connections(listener, acceptor, debug);
            let connections = hyper::server::accept::from_stream(ReceiverStream::new(connections));

            Ok(ready
               .and_then(|_| hyper::Server::builder(connections)
                    .serve(rest_server)
                    .with_graceful_shutdown(server::shutdown_future())
                    .map_err(Error::from)
                )
                .map_err(|err| eprintln!("server error: {}", err))
                .map(|_| ())
            )
        },
        "proxmox-backup-proxy.service",
    );

    server::write_pid(pbs_buildcfg::PROXMOX_BACKUP_PROXY_PID_FN)?;
    daemon::systemd_notify(daemon::SystemdNotify::Ready)?;

    let init_result: Result<(), Error> = try_block!({
        server::register_task_control_commands(&mut commando_sock)?;
        commando_sock.spawn()?;
        server::server_state_init()?;
        Ok(())
    });

    if let Err(err) = init_result {
        bail!("unable to start daemon - {}", err);
    }

    start_task_scheduler();
    start_stat_generator();

    server.await?;
    log::info!("server shutting down, waiting for active workers to complete");
    proxmox_backup::server::last_worker_future().await?;
    log::info!("done - exit server");

    Ok(())
}

fn make_tls_acceptor() -> Result<SslAcceptor, Error> {
    let key_path = configdir!("/proxy.key");
    let cert_path = configdir!("/proxy.pem");

    let mut acceptor = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls()).unwrap();
    acceptor.set_private_key_file(key_path, SslFiletype::PEM)
        .map_err(|err| format_err!("unable to read proxy key {} - {}", key_path, err))?;
    acceptor.set_certificate_chain_file(cert_path)
        .map_err(|err| format_err!("unable to read proxy cert {} - {}", cert_path, err))?;
    acceptor.check_private_key().unwrap();

    Ok(acceptor.build())
}

type ClientStreamResult =
    Result<std::pin::Pin<Box<tokio_openssl::SslStream<tokio::net::TcpStream>>>, Error>;
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
        let (sock, _addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(err) =>  {
                eprintln!("error accepting tcp connection: {}", err);
                continue;
            }
        };

        sock.set_nodelay(true).unwrap();
        let _ = set_tcp_keepalive(sock.as_raw_fd(), PROXMOX_BACKUP_TCP_KEEPALIVE_TIME);

        let ssl = { // limit acceptor_guard scope
            // Acceptor can be reloaded using the command socket "reload-certificate" command
            let acceptor_guard = acceptor.lock().unwrap();

            match openssl::ssl::Ssl::new(acceptor_guard.context()) {
                Ok(ssl) => ssl,
                Err(err) => {
                    eprintln!("failed to create Ssl object from Acceptor context - {}", err);
                    continue;
                },
            }
        };

        let stream = match tokio_openssl::SslStream::new(ssl, sock) {
            Ok(stream) => stream,
            Err(err) => {
                eprintln!("failed to create SslStream using ssl and connection socket - {}", err);
                continue;
            },
        };

        let mut stream = Box::pin(stream);
        let sender = sender.clone();

        if Arc::strong_count(&accept_counter) > MAX_PENDING_ACCEPTS {
            eprintln!("connection rejected - to many open connections");
            continue;
        }

        let accept_counter = Arc::clone(&accept_counter);
        tokio::spawn(async move {
            let accept_future = tokio::time::timeout(
                Duration::new(10, 0), stream.as_mut().accept());

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
    let abort_future = server::shutdown_future();
    let future = Box::pin(run_stat_generator());
    let task = futures::future::select(future, abort_future);
    tokio::spawn(task.map(|_| ()));
}

fn start_task_scheduler() {
    let abort_future = server::shutdown_future();
    let future = Box::pin(run_task_scheduler());
    let task = futures::future::select(future, abort_future);
    tokio::spawn(task.map(|_| ()));
}

use std::time::{SystemTime, Instant, Duration, UNIX_EPOCH};

fn next_minute() -> Result<Instant, Error> {
    let now = SystemTime::now();
    let epoch_now = now.duration_since(UNIX_EPOCH)?;
    let epoch_next = Duration::from_secs((epoch_now.as_secs()/60  + 1)*60);
    Ok(Instant::now() + epoch_next - epoch_now)
}

async fn run_task_scheduler() {

    let mut count: usize = 0;

    loop {
        count += 1;

        let delay_target = match next_minute() {  // try to run very minute
            Ok(d) => d,
            Err(err) => {
                eprintln!("task scheduler: compute next minute failed - {}", err);
                tokio::time::sleep_until(tokio::time::Instant::from_std(Instant::now() + Duration::from_secs(60))).await;
                continue;
            }
        };

        if count > 2 { // wait 1..2 minutes before starting
            match schedule_tasks().catch_unwind().await {
                Err(panic) => {
                    match panic.downcast::<&str>() {
                        Ok(msg) => {
                            eprintln!("task scheduler panic: {}", msg);
                        }
                        Err(_) => {
                            eprintln!("task scheduler panic - unknown type");
                        }
                    }
                }
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

    use proxmox_backup::config::{
        datastore::{
            self,
            DataStoreConfig,
        },
    };

    let config = match datastore::config() {
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

        let event = match parse_calendar_event(&event_str) {
            Ok(event) => event,
            Err(err) => {
                eprintln!("unable to parse schedule '{}' - {}", event_str, err);
                continue;
            }
        };

        if datastore.garbage_collection_running() { continue; }

        let worker_type = "garbage_collection";

        let last = match jobstate::last_run_time(worker_type, &store) {
            Ok(time) => time,
            Err(err) => {
                eprintln!("could not get last run time of {} {}: {}", worker_type, store, err);
                continue;
            }
        };

        let next = match compute_next_event(&event, last, false) {
            Ok(Some(next)) => next,
            Ok(None) => continue,
            Err(err) => {
                eprintln!("compute_next_event for '{}' failed - {}", event_str, err);
                continue;
            }
        };

        let now = proxmox::tools::time::epoch_i64();

        if next > now  { continue; }

        let job = match Job::new(worker_type, &store) {
            Ok(job) => job,
            Err(_) => continue, // could not get lock
        };

        let auth_id = Authid::root_auth_id();

        if let Err(err) = crate::server::do_garbage_collection_job(job, datastore, auth_id, Some(event_str), false) {
            eprintln!("unable to start garbage collection job on datastore {} - {}", store, err);
        }
    }
}

async fn schedule_datastore_prune() {

    use pbs_datastore::prune::PruneOptions;
    use proxmox_backup::{
        config::datastore::{
            self,
            DataStoreConfig,
        },
    };

    let config = match datastore::config() {
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
            keep_daily:  store_config.keep_daily,
            keep_weekly: store_config.keep_weekly,
            keep_monthly: store_config.keep_monthly,
            keep_yearly: store_config.keep_yearly,
        };

        if !prune_options.keeps_something() { // no prune settings - keep all
            continue;
        }

        let worker_type = "prune";
        if check_schedule(worker_type, &event_str, &store) {
            let job = match Job::new(worker_type, &store) {
                Ok(job) => job,
                Err(_) => continue, // could not get lock
            };

            let auth_id = Authid::root_auth_id().clone();
            if let Err(err) = do_prune_job(job, prune_options, store.clone(), &auth_id, Some(event_str)) {
                eprintln!("unable to start datastore prune job {} - {}", &store, err);
            }
        };
    }
}

async fn schedule_datastore_sync_jobs() {

    use proxmox_backup::config::sync::{
        self,
        SyncJobConfig,
    };

    let config = match sync::config() {
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
            if let Err(err) = do_sync_job(job, job_config, &auth_id, Some(event_str)) {
                eprintln!("unable to start datastore sync job {} - {}", &job_id, err);
            }
        };
    }
}

async fn schedule_datastore_verify_jobs() {

    use proxmox_backup::config::verify::{
        self,
        VerificationJobConfig,
    };

    let config = match verify::config() {
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
            let job = match Job::new(&worker_type, &job_id) {
                Ok(job) => job,
                Err(_) => continue, // could not get lock
            };
            if let Err(err) = do_verification_job(job, job_config, &auth_id, Some(event_str)) {
                eprintln!("unable to start datastore verification job {} - {}", &job_id, err);
            }
        };
    }
}

async fn schedule_tape_backup_jobs() {

    use proxmox_backup::config::tape_job::{
        self,
        TapeBackupJobConfig,
    };

    let config = match tape_job::config() {
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
            let job = match Job::new(&worker_type, &job_id) {
                Ok(job) => job,
                Err(_) => continue, // could not get lock
            };
            if let Err(err) = do_tape_backup_job(job, job_config.setup, &auth_id, Some(event_str)) {
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
                jobstate::JobState::Created { .. } => {},
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
        Authid::root_auth_id().clone(),
        false,
        move |worker| {
            job.start(&worker.upid().to_string())?;
            worker.log("starting task log rotation".to_string());

            let result = try_block!({
                let max_size = 512 * 1024 - 1; // an entry has ~ 100b, so > 5000 entries/file
                let max_files = 20; // times twenty files gives > 100000 task entries
                let has_rotated = rotate_task_log_archive(max_size, true, Some(max_files))?;
                if has_rotated {
                    worker.log("task log archive was rotated".to_string());
                } else {
                    worker.log("task log archive was not rotated".to_string());
                }

                let max_size = 32 * 1024 * 1024 - 1;
                let max_files = 14;
                let mut logrotate = LogRotate::new(pbs_buildcfg::API_ACCESS_LOG_FN, true)
                        .ok_or_else(|| format_err!("could not get API access log file names"))?;

                if logrotate.rotate(max_size, None, Some(max_files))? {
                    println!("rotated access log, telling daemons to re-open log file");
                    pbs_runtime::block_on(command_reopen_logfiles())?;
                    worker.log("API access log was rotated".to_string());
                } else {
                    worker.log("API access log was not rotated".to_string());
                }

                let mut logrotate = LogRotate::new(pbs_buildcfg::API_AUTH_LOG_FN, true)
                        .ok_or_else(|| format_err!("could not get API auth log file names"))?;

                if logrotate.rotate(max_size, None, Some(max_files))? {
                    worker.log("API authentication log was rotated".to_string());
                } else {
                    worker.log("API authentication log was not rotated".to_string());
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

async fn command_reopen_logfiles() -> Result<(), Error> {
    // only care about the most recent daemon instance for each, proxy & api, as other older ones
    // should not respond to new requests anyway, but only finish their current one and then exit.
    let sock = server::our_ctrl_sock();
    let f1 = server::send_command(sock, "{\"command\":\"api-access-log-reopen\"}\n");

    let pid = server::read_pid(pbs_buildcfg::PROXMOX_BACKUP_API_PID_FN)?;
    let sock = server::ctrl_sock_from_pid(pid);
    let f2 = server::send_command(sock, "{\"command\":\"api-access-log-reopen\"}\n");

    match futures::join!(f1, f2) {
        (Err(e1), Err(e2)) => Err(format_err!("reopen commands failed, proxy: {}; api: {}", e1, e2)),
        (Err(e1), Ok(_)) => Err(format_err!("reopen commands failed, proxy: {}", e1)),
        (Ok(_), Err(e2)) => Err(format_err!("reopen commands failed, api: {}", e2)),
        _ => Ok(()),
    }
}

async fn run_stat_generator() {

    let mut count = 0;
    loop {
        count += 1;
        let save = if count >= 6 { count = 0; true } else { false };

        let delay_target = Instant::now() +  Duration::from_secs(10);

        generate_host_stats(save).await;

        tokio::time::sleep_until(tokio::time::Instant::from_std(delay_target)).await;

     }

}

fn rrd_update_gauge(name: &str, value: f64, save: bool) {
    use proxmox_backup::rrd;
    if let Err(err) = rrd::update_value(name, value, rrd::DST::Gauge, save) {
        eprintln!("rrd::update_value '{}' failed - {}", name, err);
    }
}

fn rrd_update_derive(name: &str, value: f64, save: bool) {
    use proxmox_backup::rrd;
    if let Err(err) = rrd::update_value(name, value, rrd::DST::Derive, save) {
        eprintln!("rrd::update_value '{}' failed - {}", name, err);
    }
}

async fn generate_host_stats(save: bool) {
    use proxmox::sys::linux::procfs::{
        read_meminfo, read_proc_stat, read_proc_net_dev, read_loadavg};
    use proxmox_backup::config::datastore;


    pbs_runtime::block_in_place(move || {

        match read_proc_stat() {
            Ok(stat) => {
                rrd_update_gauge("host/cpu", stat.cpu, save);
                rrd_update_gauge("host/iowait", stat.iowait_percent, save);
            }
            Err(err) => {
                eprintln!("read_proc_stat failed - {}", err);
            }
        }

        match read_meminfo() {
            Ok(meminfo) => {
                rrd_update_gauge("host/memtotal", meminfo.memtotal as f64, save);
                rrd_update_gauge("host/memused", meminfo.memused as f64, save);
                rrd_update_gauge("host/swaptotal", meminfo.swaptotal as f64, save);
                rrd_update_gauge("host/swapused", meminfo.swapused as f64, save);
            }
            Err(err) => {
                eprintln!("read_meminfo failed - {}", err);
            }
        }

        match read_proc_net_dev() {
            Ok(netdev) => {
                use proxmox_backup::config::network::is_physical_nic;
                let mut netin = 0;
                let mut netout = 0;
                for item in netdev {
                    if !is_physical_nic(&item.device) { continue; }
                    netin += item.receive;
                    netout += item.send;
                }
                rrd_update_derive("host/netin", netin as f64, save);
                rrd_update_derive("host/netout", netout as f64, save);
            }
            Err(err) => {
                eprintln!("read_prox_net_dev failed - {}", err);
            }
        }

        match read_loadavg() {
            Ok(loadavg) => {
                rrd_update_gauge("host/loadavg", loadavg.0 as f64, save);
            }
            Err(err) => {
                eprintln!("read_loadavg failed - {}", err);
            }
        }

        let disk_manager = DiskManage::new();

        gather_disk_stats(disk_manager.clone(), Path::new("/"), "host", save);

        match datastore::config() {
            Ok((config, _)) => {
                let datastore_list: Vec<datastore::DataStoreConfig> =
                    config.convert_to_typed_array("datastore").unwrap_or_default();

                for config in datastore_list {

                    let rrd_prefix = format!("datastore/{}", config.name);
                    let path = std::path::Path::new(&config.path);
                    gather_disk_stats(disk_manager.clone(), path, &rrd_prefix, save);
                }
            }
            Err(err) => {
                eprintln!("read datastore config failed - {}", err);
            }
        }

    });
}

fn check_schedule(worker_type: &str, event_str: &str, id: &str) -> bool {
    let event = match parse_calendar_event(event_str) {
        Ok(event) => event,
        Err(err) => {
            eprintln!("unable to parse schedule '{}' - {}", event_str, err);
            return false;
        }
    };

    let last = match jobstate::last_run_time(worker_type, &id) {
        Ok(time) => time,
        Err(err) => {
            eprintln!("could not get last run time of {} {}: {}", worker_type, id, err);
            return false;
        }
    };

    let next = match compute_next_event(&event, last, false) {
        Ok(Some(next)) => next,
        Ok(None) => return false,
        Err(err) => {
            eprintln!("compute_next_event for '{}' failed - {}", event_str, err);
            return false;
        }
    };

    let now = proxmox::tools::time::epoch_i64();
    next <= now
}

fn gather_disk_stats(disk_manager: Arc<DiskManage>, path: &Path, rrd_prefix: &str, save: bool) {

    match proxmox_backup::tools::disks::disk_usage(path) {
        Ok(status) => {
            let rrd_key = format!("{}/total", rrd_prefix);
            rrd_update_gauge(&rrd_key, status.total as f64, save);
            let rrd_key = format!("{}/used", rrd_prefix);
            rrd_update_gauge(&rrd_key, status.used as f64, save);
        }
        Err(err) => {
            eprintln!("read disk_usage on {:?} failed - {}", path, err);
        }
    }

    match disk_manager.find_mounted_device(path) {
        Ok(None) => {},
        Ok(Some((fs_type, device, source))) => {
            let mut device_stat = None;
            match fs_type.as_str() {
                "zfs" => {
                    if let Some(source) = source {
                        let pool = get_pool_from_dataset(&source).unwrap_or(&source);
                        match zfs_pool_stats(pool) {
                            Ok(stat) => device_stat = stat,
                            Err(err) => eprintln!("zfs_pool_stats({:?}) failed - {}", pool, err),
                        }
                    }
                }
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
                rrd_update_derive(&rrd_key, stat.read_ios as f64, save);
                let rrd_key = format!("{}/read_bytes", rrd_prefix);
                rrd_update_derive(&rrd_key, (stat.read_sectors*512) as f64, save);

                let rrd_key = format!("{}/write_ios", rrd_prefix);
                rrd_update_derive(&rrd_key, stat.write_ios as f64, save);
                let rrd_key = format!("{}/write_bytes", rrd_prefix);
                rrd_update_derive(&rrd_key, (stat.write_sectors*512) as f64, save);

                let rrd_key = format!("{}/io_ticks", rrd_prefix);
                rrd_update_derive(&rrd_key, (stat.io_ticks as f64)/1000.0, save);
            }
        }
        Err(err) => {
            eprintln!("find_mounted_device failed - {}", err);
        }
    }
}
