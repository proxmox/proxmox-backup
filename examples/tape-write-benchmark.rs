use std::{
    fs::File,
    io::Read,
    time::{Duration, SystemTime},
};

use anyhow::{format_err, Error};
use pbs_tape::TapeWrite;
use proxmox_backup::tape::drive::{LtoTapeHandle, TapeDriver};

const URANDOM_PATH: &str = "/dev/urandom";
const CHUNK_SIZE: usize = 4 * 1024 * 1024; // 4 MiB
const LOG_LIMIT: usize = 4 * 1024 * 1024 * 1024; // 4 GiB

fn write_chunks<'a>(
    mut writer: Box<dyn 'a + TapeWrite>,
    blob_size: usize,
    max_size: usize,
    max_time: Duration,
) -> Result<(), Error> {
    // prepare chunks in memory

    let mut blob: Vec<u8> = vec![0u8; blob_size];

    let mut file = File::open(URANDOM_PATH)?;
    file.read_exact(&mut blob[..])?;

    let start_time = SystemTime::now();
    loop {
        let iteration_time = SystemTime::now();
        let mut count = 0;
        let mut bytes_written = 0;
        let mut idx = 0;
        let mut incr_count = 0;
        loop {
            if writer.write_all(&blob)? {
                eprintln!("LEOM reached");
                break;
            }

            // modifying chunks a bit to mitigate compression/deduplication
            blob[idx] = blob[idx].wrapping_add(1);
            incr_count += 1;
            if incr_count >= 256 {
                incr_count = 0;
                idx += 1;
            }
            count += 1;
            bytes_written += blob_size;

            if bytes_written > max_size {
                break;
            }
        }

        let elapsed = iteration_time.elapsed()?.as_secs_f64();
        let elapsed_total = start_time.elapsed()?;
        eprintln!(
            "{:.2}s: wrote {} chunks ({:.2} MB at {:.2} MB/s, average: {:.2} MB/s)",
            elapsed_total.as_secs_f64(),
            count,
            bytes_written as f64 / 1_000_000.0,
            (bytes_written as f64) / (1_000_000.0 * elapsed),
            (writer.bytes_written() as f64) / (1_000_000.0 * elapsed_total.as_secs_f64()),
        );

        if elapsed_total > max_time {
            break;
        }
    }

    Ok(())
}
fn main() -> Result<(), Error> {
    let mut args = std::env::args_os();
    args.next(); // binary name
    let path = args.next().expect("no path to tape device given");
    let file = File::open(path).map_err(|err| format_err!("could not open tape device: {err}"))?;
    let mut drive = LtoTapeHandle::new(file)
        .map_err(|err| format_err!("error creating drive handle: {err}"))?;
    write_chunks(
        drive
            .write_file()
            .map_err(|err| format_err!("error starting file write: {err}"))?,
        CHUNK_SIZE,
        LOG_LIMIT,
        Duration::new(60 * 20, 0),
    )
    .map_err(|err| format_err!("error writing data to tape: {err}"))?;
    Ok(())
}
