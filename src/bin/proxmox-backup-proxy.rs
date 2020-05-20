use std::sync::Arc;

use anyhow::{bail, format_err, Error};
use futures::*;
use hyper;
use openssl::ssl::{SslMethod, SslAcceptor, SslFiletype};

use proxmox::try_block;
use proxmox::api::RpcEnvironmentType;

use proxmox_backup::configdir;
use proxmox_backup::buildcfg;
use proxmox_backup::server;
use proxmox_backup::tools::daemon;
use proxmox_backup::server::{ApiConfig, rest::*};
use proxmox_backup::auth_helpers::*;

fn main() {
    if let Err(err) = proxmox_backup::tools::runtime::main(run()) {
        eprintln!("Error: {}", err);
        std::process::exit(-1);
    }
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

    // add default dirs which includes jquery and bootstrap
    // my $base = '/usr/share/libpve-http-server-perl';
    // add_dirs($self->{dirs}, '/css/' => "$base/css/");
    // add_dirs($self->{dirs}, '/js/' => "$base/js/");
    // add_dirs($self->{dirs}, '/fonts/' => "$base/fonts/");
    config.add_alias("novnc", "/usr/share/novnc-pve");
    config.add_alias("extjs", "/usr/share/javascript/extjs");
    config.add_alias("fontawesome", "/usr/share/fonts-font-awesome");
    config.add_alias("xtermjs", "/usr/share/pve-xtermjs");
    config.add_alias("widgettoolkit", "/usr/share/javascript/proxmox-widget-toolkit");
    config.add_alias("css", "/usr/share/javascript/proxmox-backup/css");
    config.add_alias("docs", "/usr/share/doc/proxmox-backup/html");

    let rest_server = RestServer::new(config);

    //openssl req -x509 -newkey rsa:4096 -keyout /etc/proxmox-backup/proxy.key -out /etc/proxmox-backup/proxy.pem -nodes
    let key_path = configdir!("/proxy.key");
    let cert_path = configdir!("/proxy.pem");

    let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
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
                        sock.set_send_buffer_size(1024*1024).unwrap();
                        sock.set_recv_buffer_size(1024*1024).unwrap();
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

    server.await?;
    log::info!("server shutting down, waiting for active workers to complete");
    proxmox_backup::server::last_worker_future().await?;
    log::info!("done - exit server");

    Ok(())
}

fn start_task_scheduler() {
    let abort_future = server::shutdown_future();
    let future = Box::pin(run_task_scheduler());
    let task = futures::future::select(future, abort_future);
    tokio::spawn(task.map(|_| ()));
}

use std::time:: {Instant, Duration, SystemTime, UNIX_EPOCH};

fn next_minute() -> Result<Instant, Error> {
    let epoch_now = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let epoch_next = Duration::from_secs((epoch_now.as_secs()/60 + 1)*60);
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

    Ok(())
}

fn lookup_last_worker(worker_type: &str, worker_id: &str) -> Result<Option<server::UPID>, Error> {

    let list = proxmox_backup::server::read_task_list()?;

    let mut last: Option<&server::UPID> = None;

    for entry in list.iter() {
        if entry.upid.worker_type == worker_type {
            if let Some(ref id) = entry.upid.worker_id {
                if id == worker_id {
                    match last {
                        Some(ref upid) => {
                            if upid.starttime < entry.upid.starttime {
                                last = Some(&entry.upid)
                            }
                        }
                        None => {
                            last = Some(&entry.upid)
                        }
                    }
                }
            }
        }
    }

    Ok(last.cloned())
}


async fn schedule_datastore_garbage_collection() {

    use proxmox_backup::backup::DataStore;
    use proxmox_backup::server::{UPID, WorkerTask};
    use proxmox_backup::config::datastore::{self, DataStoreConfig};
    use proxmox_backup::tools::systemd::time::{
        parse_calendar_event, compute_next_event};

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
            match lookup_last_worker(worker_type, &store) {
                Ok(Some(upid)) => upid.starttime,
                Ok(None) => 0,
                Err(err) => {
                    eprintln!("lookup_last_job_start failed: {}", err);
                    continue;
                }
            }
        };

        let next = match compute_next_event(&event, last, false) {
            Ok(next) => next,
            Err(err) => {
                eprintln!("compute_next_event for '{}' failed - {}", event_str, err);
                continue;
            }
        };
        let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(epoch_now) => epoch_now.as_secs() as i64,
            Err(err) => {
                eprintln!("query system time failed - {}", err);
                continue;
            }
        };
        if next > now  { continue; }

        let store2 = store.clone();

        if let Err(err) = WorkerTask::new_thread(
            worker_type,
            Some(store.clone()),
            "root@pam",
            false,
            move |worker| {
                worker.log(format!("starting garbage collection on store {}", store));
                worker.log(format!("task triggered by schedule '{}'", event_str));
                datastore.garbage_collection(&worker)
            }
        ) {
            eprintln!("unable to start garbage collection on store {} - {}", store2, err);
        }
    }
}

async fn schedule_datastore_prune() {

    use proxmox_backup::backup::{
        PruneOptions, DataStore, BackupGroup, BackupDir, compute_prune_info};
    use proxmox_backup::server::{WorkerTask};
    use proxmox_backup::config::datastore::{self, DataStoreConfig};
    use proxmox_backup::tools::systemd::time::{
        parse_calendar_event, compute_next_event};

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

        //fixme: if last_prune_job_stzill_running { continue; }

        let worker_type = "prune";

        let last = match lookup_last_worker(worker_type, &store) {
            Ok(Some(upid)) => upid.starttime,
            Ok(None) => 0,
            Err(err) => {
                eprintln!("lookup_last_job_start failed: {}", err);
                continue;
            }
        };

        let next = match compute_next_event(&event, last, false) {
            Ok(next) => next,
            Err(err) => {
                eprintln!("compute_next_event for '{}' failed - {}", event_str, err);
                continue;
            }
        };

        let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(epoch_now) => epoch_now.as_secs() as i64,
            Err(err) => {
                eprintln!("query system time failed - {}", err);
                continue;
            }
        };
        if next > now  { continue; }

        let store2 = store.clone();

        if let Err(err) = WorkerTask::new_thread(
            worker_type,
            Some(store.clone()),
            "root@pam",
            false,
            move |worker| {
                worker.log(format!("Starting datastore prune on store \"{}\"", store));
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
                            BackupDir::backup_time_to_string(info.backup_dir.backup_time())));

                        if !keep {
                            datastore.remove_backup_dir(&info.backup_dir)?;
                        }
                    }
                }

                Ok(())
            }
        ) {
            eprintln!("unable to start datastore prune on store {} - {}", store2, err);
        }
    }
}
