use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use anyhow::{bail, Error};
use futures::*;
use serde_json::{json, Value};
use tokio::signal::unix::{signal, SignalKind};

use proxmox_router::cli::format_and_print_result;

use pbs_api_types::percent_encoding::percent_encode_component;

use super::HttpClient;

/// Display task log on console
///
/// This polls the task API and prints the log to the console. It also
/// catches interrupt signals, and sends a abort request to the task if
/// the user presses CTRL-C. Two interrupts cause an immediate end of
/// the loop. The task may still run in that case.
pub async fn display_task_log(
    client: &HttpClient,
    upid_str: &str,
    strip_date: bool,
) -> Result<(), Error> {
    let mut signal_stream = signal(SignalKind::interrupt())?;
    let abort_count = Arc::new(AtomicUsize::new(0));
    let abort_count2 = Arc::clone(&abort_count);

    let abort_future = async move {
        while signal_stream.recv().await.is_some() {
            log::info!("got shutdown request (SIGINT)");
            let prev_count = abort_count2.fetch_add(1, Ordering::SeqCst);
            if prev_count >= 1 {
                log::info!("forced exit (task still running)");
                break;
            }
        }
        Ok::<_, Error>(())
    };

    let request_future = async move {
        let mut start = 1;
        let limit = 500;

        loop {
            let abort = abort_count.load(Ordering::Relaxed);
            if abort > 0 {
                let path = format!(
                    "api2/json/nodes/localhost/tasks/{}",
                    percent_encode_component(upid_str)
                );
                let _ = client.delete(&path, None).await?;
            }

            let param = json!({ "start": start, "limit": limit, "test-status": true });

            let path = format!(
                "api2/json/nodes/localhost/tasks/{}/log",
                percent_encode_component(upid_str)
            );
            let result = client.get(&path, Some(param)).await?;

            let active = result["active"].as_bool().unwrap();
            let total = result["total"].as_u64().unwrap();
            let data = result["data"].as_array().unwrap();

            let lines = data.len();

            for item in data {
                let n = item["n"].as_u64().unwrap();
                let t = item["t"].as_str().unwrap();
                if n != start {
                    bail!("got wrong line number in response data ({} != {}", n, start);
                }
                if strip_date && t.len() > 27 && &t[25..27] == ": " {
                    let line = &t[27..];
                    println!("{}", line);
                } else {
                    println!("{}", t);
                }
                start += 1;
            }

            if start > total {
                if active {
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                } else {
                    break;
                }
            } else if lines != limit {
                bail!(
                    "got wrong number of lines from server ({} != {})",
                    lines,
                    limit
                );
            }
        }

        Ok(())
    };

    futures::select! {
        request = request_future.fuse() => request?,
        abort = abort_future.fuse() => abort?,
    };

    Ok(())
}

/// Display task result (upid), or view task log - depending on output format
pub async fn view_task_result(
    client: &HttpClient,
    result: Value,
    output_format: &str,
) -> Result<(), Error> {
    let data = &result["data"];
    if output_format == "text" {
        if let Some(upid) = data.as_str() {
            display_task_log(client, upid, true).await?;
        }
    } else {
        format_and_print_result(data, output_format);
    }

    Ok(())
}
