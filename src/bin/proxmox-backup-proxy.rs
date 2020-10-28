use std::sync::{Arc};
use std::path::{Path, PathBuf};
use std::os::unix::io::AsRawFd;

use anyhow::{bail, format_err, Error};
use futures::*;
use hyper;
use openssl::ssl::{SslMethod, SslAcceptor, SslFiletype};

use proxmox::try_block;
use proxmox::api::RpcEnvironmentType;

use proxmox_backup::{
    backup::DataStore,
    server::{
        UPID,
        WorkerTask,
        ApiConfig,
        rest::*,
        jobstate::{
            self,
            Job,
        },
        rotate_task_log_archive,
    },
    tools::systemd::time::{
        parse_calendar_event,
        compute_next_event,
    },
};


use proxmox_backup::api2::types::Userid;
use proxmox_backup::configdir;
use proxmox_backup::buildcfg;
use proxmox_backup::server;
use proxmox_backup::auth_helpers::*;
use proxmox_backup::tools::{
    daemon,
    disks::{
        DiskManage,
        zfs_pool_stats,
    },
    socket::{
        set_tcp_keepalive,
        PROXMOX_BACKUP_TCP_KEEPALIVE_TIME,
    },
};

use proxmox_backup::api2::pull::do_sync_job;
use proxmox_backup::server::do_verification_job;

fn main() -> Result<(), Error> {
    proxmox_backup::tools::setup_safe_path_env();

    let backup_uid = proxmox_backup::backup::backup_user()?.uid;
    let backup_gid = proxmox_backup::backup::backup_group()?.gid;
    let running_uid = nix::unistd::Uid::effective();
    let running_gid = nix::unistd::Gid::effective();

    if running_uid != backup_uid || running_gid != backup_gid {
        bail!("proxy not running as backup user or group (got uid {} gid {})", running_uid, running_gid);
    }

    proxmox_backup::tools::runtime::main(run())
}

async fn run() -> Result<(), Error> {
    if let Err(err) = syslog::init(
        syslog::Facility::LOG_DAEMON,
        log::LevelFilter::Info,
        Some("proxmox-backup-proxy")) {
        bail!("unable to inititialize syslog - {}", err);
    }

    let _ = public_auth_key(); // load with lazy_static
    let _ = csrf_secret(); // load with lazy_static

    let mut config = ApiConfig::new(
        buildcfg::JS_DIR, &proxmox_backup::api2::ROUTER, RpcEnvironmentType::PUBLIC)?;

    config.add_alias("novnc", "/usr/share/novnc-pve");
    config.add_alias("extjs", "/usr/share/javascript/extjs");
    config.add_alias("fontawesome", "/usr/share/fonts-font-awesome");
    config.add_alias("xtermjs", "/usr/share/pve-xtermjs");
    config.add_alias("locale", "/usr/share/pbs-i18n");
    config.add_alias("widgettoolkit", "/usr/share/javascript/proxmox-widget-toolkit");
    config.add_alias("css", "/usr/share/javascript/proxmox-backup/css");
    config.add_alias("docs", "/usr/share/doc/proxmox-backup/html");

    let mut indexpath = PathBuf::from(buildcfg::JS_DIR);
    indexpath.push("index.hbs");
    config.register_template("index", &indexpath)?;
    config.register_template("console", "/usr/share/pve-xtermjs/index.html.hbs")?;

    config.enable_file_log(buildcfg::API_ACCESS_LOG_FN)?;

    let rest_server = RestServer::new(config);

    //openssl req -x509 -newkey rsa:4096 -keyout /etc/proxmox-backup/proxy.key -out /etc/proxmox-backup/proxy.pem -nodes
    let key_path = configdir!("/proxy.key");
    let cert_path = configdir!("/proxy.pem");

    let mut acceptor = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls()).unwrap();
    acceptor.set_private_key_file(key_path, SslFiletype::PEM)
        .map_err(|err| format_err!("unable to read proxy key {} - {}", key_path, err))?;
    acceptor.set_certificate_chain_file(cert_path)
        .map_err(|err| format_err!("unable to read proxy cert {} - {}", cert_path, err))?;
    acceptor.check_private_key().unwrap();

    let acceptor = Arc::new(acceptor.build());

    let server = daemon::create_daemon(
        ([0,0,0,0,0,0,0,0], 8007).into(),
        |listener, ready| {
            let connections = proxmox_backup::tools::async_io::StaticIncoming::from(listener)
                .map_err(Error::from)
                .try_filter_map(move |(sock, _addr)| {
                    let acceptor = Arc::clone(&acceptor);
                    async move {
                        sock.set_nodelay(true).unwrap();

                        let _ = set_tcp_keepalive(sock.as_raw_fd(), PROXMOX_BACKUP_TCP_KEEPALIVE_TIME);

                        Ok(tokio_openssl::accept(&acceptor, sock)
                            .await
                            .ok() // handshake errors aren't be fatal, so return None to filter
                        )
                    }
                });
            let connections = proxmox_backup::tools::async_io::HyperAccept(connections);

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
    );

    daemon::systemd_notify(daemon::SystemdNotify::Ready)?;

    let init_result: Result<(), Error> = try_block!({
        server::create_task_control_socket()?;
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
                tokio::time::delay_until(tokio::time::Instant::from_std(Instant::now() + Duration::from_secs(60))).await;
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

        tokio::time::delay_until(tokio::time::Instant::from_std(delay_target)).await;
    }
}

async fn schedule_tasks() -> Result<(), Error> {

    schedule_datastore_garbage_collection().await;
    schedule_datastore_prune().await;
    schedule_datastore_sync_jobs().await;
    schedule_datastore_verify_jobs().await;
    schedule_task_log_rotate().await;

    Ok(())
}

async fn schedule_datastore_garbage_collection() {

    use proxmox_backup::config::datastore::{
        self,
        DataStoreConfig,
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

        let stat = datastore.last_gc_status();
        let last = if let Some(upid_str) = stat.upid {
            match upid_str.parse::<UPID>() {
                Ok(upid) => upid.starttime,
                Err(err) => {
                    eprintln!("unable to parse upid '{}' - {}", upid_str, err);
                    continue;
                }
            }
        } else {
            match jobstate::last_run_time(worker_type, &store) {
                Ok(time) => time,
                Err(err) => {
                    eprintln!("could not get last run time of {} {}: {}", worker_type, store, err);
                    continue;
                }
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

        let mut job = match Job::new(worker_type, &store) {
            Ok(job) => job,
            Err(_) => continue, // could not get lock
        };

        let store2 = store.clone();

        if let Err(err) = WorkerTask::new_thread(
            worker_type,
            Some(store.clone()),
            Userid::backup_userid().clone(),
            false,
            move |worker| {
                job.start(&worker.upid().to_string())?;

                worker.log(format!("starting garbage collection on store {}", store));
                worker.log(format!("task triggered by schedule '{}'", event_str));

                let result = datastore.garbage_collection(&*worker, worker.upid());

                let status = worker.create_state(&result);

                if let Err(err) = job.finish(status) {
                    eprintln!("could not finish job state for {}: {}", worker_type, err);
                }

                result
            }
        ) {
            eprintln!("unable to start garbage collection on store {} - {}", store2, err);
        }
    }
}

async fn schedule_datastore_prune() {

    use proxmox_backup::{
        backup::{
            PruneOptions,
            BackupGroup,
            compute_prune_info,
        },
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
        let datastore = match DataStore::lookup_datastore(&store) {
            Ok(datastore) => datastore,
            Err(err) => {
                eprintln!("lookup_datastore '{}' failed - {}", store, err);
                continue;
            }
        };

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

        let event = match parse_calendar_event(&event_str) {
            Ok(event) => event,
            Err(err) => {
                eprintln!("unable to parse schedule '{}' - {}", event_str, err);
                continue;
            }
        };

        let worker_type = "prune";

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

        let mut job = match Job::new(worker_type, &store) {
            Ok(job) => job,
            Err(_) => continue, // could not get lock
        };

        let store2 = store.clone();

        if let Err(err) = WorkerTask::new_thread(
            worker_type,
            Some(store.clone()),
            Userid::backup_userid().clone(),
            false,
            move |worker| {

                job.start(&worker.upid().to_string())?;

                let result = try_block!({

                    worker.log(format!("Starting datastore prune on store \"{}\"", store));
                    worker.log(format!("task triggered by schedule '{}'", event_str));
                    worker.log(format!("retention options: {}", prune_options.cli_options_string()));

                    let base_path = datastore.base_path();

                    let groups = BackupGroup::list_groups(&base_path)?;
                    for group in groups {
                        let list = group.list_backups(&base_path)?;
                        let mut prune_info = compute_prune_info(list, &prune_options)?;
                        prune_info.reverse(); // delete older snapshots first

                        worker.log(format!("Starting prune on store \"{}\" group \"{}/{}\"",
                                store, group.backup_type(), group.backup_id()));

                        for (info, keep) in prune_info {
                            worker.log(format!(
                                    "{} {}/{}/{}",
                                    if keep { "keep" } else { "remove" },
                                    group.backup_type(), group.backup_id(),
                                    info.backup_dir.backup_time_string()));
                            if !keep {
                                datastore.remove_backup_dir(&info.backup_dir, true)?;
                            }
                        }
                    }
                    Ok(())
                });

                let status = worker.create_state(&result);

                if let Err(err) = job.finish(status) {
                    eprintln!("could not finish job state for {}: {}", worker_type, err);
                }

                result
            }
        ) {
            eprintln!("unable to start datastore prune on store {} - {}", store2, err);
        }
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

        let event = match parse_calendar_event(&event_str) {
            Ok(event) => event,
            Err(err) => {
                eprintln!("unable to parse schedule '{}' - {}", event_str, err);
                continue;
            }
        };

        let worker_type = "syncjob";

        let last = match jobstate::last_run_time(worker_type, &job_id) {
            Ok(time) => time,
            Err(err) => {
                eprintln!("could not get last run time of {} {}: {}", worker_type, job_id, err);
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

        let job = match Job::new(worker_type, &job_id) {
            Ok(job) => job,
            Err(_) => continue, // could not get lock
        };

        let userid = Userid::backup_userid().clone();

        if let Err(err) = do_sync_job(job, job_config, &userid, Some(event_str)) {
            eprintln!("unable to start datastore sync job {} - {}", &job_id, err);
        }
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
        let event = match parse_calendar_event(&event_str) {
            Ok(event) => event,
            Err(err) => {
                eprintln!("unable to parse schedule '{}' - {}", event_str, err);
                continue;
            }
        };
        let worker_type = "verificationjob";
        let last = match jobstate::last_run_time(worker_type, &job_id) {
            Ok(time) => time,
            Err(err) => {
                eprintln!("could not get last run time of {} {}: {}", worker_type, job_id, err);
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
        if next > now { continue; }
        let job = match Job::new(worker_type, &job_id) {
            Ok(job) => job,
            Err(_) => continue, // could not get lock
        };
        let userid = Userid::backup_userid().clone();
        if let Err(err) = do_verification_job(job, job_config, &userid, Some(event_str)) {
            eprintln!("unable to start datastore verification job {} - {}", &job_id, err);
        }
    }
}

async fn schedule_task_log_rotate() {

    let worker_type = "logrotate";
    let job_id = "task_archive";

    let last = match jobstate::last_run_time(worker_type, job_id) {
        Ok(time) => time,
        Err(err) => {
            eprintln!("could not get last run time of task log archive rotation: {}", err);
            return;
        }
    };

    // schedule daily at 00:00 like normal logrotate
    let schedule = "00:00";

    let event = match parse_calendar_event(schedule) {
        Ok(event) => event,
        Err(err) => {
            // should not happen?
            eprintln!("unable to parse schedule '{}' - {}", schedule, err);
            return;
        }
    };

    let next = match compute_next_event(&event, last, false) {
        Ok(Some(next)) => next,
        Ok(None) => return,
        Err(err) => {
            eprintln!("compute_next_event for '{}' failed - {}", schedule, err);
            return;
        }
    };

    let now = proxmox::tools::time::epoch_i64();

    if next > now {
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
        Some(job_id.to_string()),
        Userid::backup_userid().clone(),
        false,
        move |worker| {
            job.start(&worker.upid().to_string())?;
            worker.log(format!("starting task log rotation"));

            let result = try_block!({
                // rotate task log archive
                let max_size = 500000; // a normal entry has about 100b, so ~ 5000 entries/file
                let max_files = 20; // times twenty files gives at least 100000 task entries
                let has_rotated = rotate_task_log_archive(max_size, true, Some(max_files))?;
                if has_rotated {
                    worker.log(format!("task log archive was rotated"));
                } else {
                    worker.log(format!("task log archive was not rotated"));
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

async fn run_stat_generator() {

    let mut count = 0;
    loop {
        count += 1;
        let save = if count >= 6 { count = 0; true } else { false };

        let delay_target = Instant::now() +  Duration::from_secs(10);

        generate_host_stats(save).await;

        tokio::time::delay_until(tokio::time::Instant::from_std(delay_target)).await;

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


    proxmox_backup::tools::runtime::block_in_place(move || {

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
                    config.convert_to_typed_array("datastore").unwrap_or(Vec::new());

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
                    if let Some(pool) = source {
                        match zfs_pool_stats(&pool) {
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
