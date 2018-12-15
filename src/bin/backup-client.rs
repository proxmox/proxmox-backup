extern crate apitest;

use failure::*;

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::io::ErrorKind;
use std::io::prelude::*;
use std::iter::Iterator;

use apitest::cli::command::*;
use apitest::api::schema::*;
use apitest::api::router::*;
use apitest::backup::chunk_store::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use apitest::config::datastore;

fn required_string_param<'a>(param: &'a Value, name: &str) -> &'a str {
    param[name].as_str().expect(&format!("missing parameter '{}'", name))
}


// Note: We cannot implement an Iterator, because Iterators cannot
// return a borrowed buffer ref (we want zero-copy)
fn file_chunker<C>(
    mut file: File,
    chunk_size: usize,
    chunk_cb: C
) -> Result<(), Error>
    where C: Fn(usize, &[u8]) -> Result<bool, Error>
{

    const read_buffer_size: usize = 4*1024*1024; // 4M

    if chunk_size > read_buffer_size { bail!("chunk size too large!"); }

    let mut buf = vec![0u8; read_buffer_size];

    let mut pos = 0;
    let mut file_pos = 0;
    loop {
        let mut eof = false;
        let mut tmp = &mut buf[..];
       // try to read large portions, at least chunk_size
        while pos < chunk_size {
            match file.read(tmp) {
                Ok(0) => { eof = true; break; },
                Ok(n) => {
                    pos += n;
                    if pos > chunk_size { break; }
                    tmp = &mut tmp[n..];
                }
                Err(ref e) if e.kind() == ErrorKind::Interrupted => { /* try again */ }
                Err(e) => bail!("read error - {}", e.to_string()),
            }
        }
        println!("READ {} {}", pos, eof);

        let mut start = 0;
        while start + chunk_size <= pos {
            if !(chunk_cb)(file_pos, &buf[start..start+chunk_size])? { break; }
            file_pos += chunk_size;
            start += chunk_size;
        }
        if eof {
            if start < pos {
                (chunk_cb)(file_pos, &buf[start..pos])?;
                //file_pos += pos - start;
            }
            break;
        } else {
            let rest = pos - start;
            if rest > 0 {
                let ptr = buf.as_mut_ptr();
                unsafe { std::ptr::copy_nonoverlapping(ptr.add(start), ptr, rest); }
                pos = rest;
            } else {
                pos = 0;
            }
        }
    }

    Ok(())

}

fn backup_file(param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    let filename = required_string_param(&param, "filename");
    let store = required_string_param(&param, "store");

    let config = datastore::config()?;
    let (_, store_config) = config.sections.get(store)
        .ok_or(format_err!("no such datastore '{}'", store))?;

    let path = store_config["path"].as_str().unwrap();

    let _store = ChunkStore::open(path)?;

    println!("Backup file '{}' to '{}'", filename, store);

    let file = std::fs::File::open(filename)?;

    file_chunker(file, 64*1024, |pos, chunk| {
        println!("CHUNK {} {}", pos, chunk.len());
        Ok(true)
    })?;

    Ok(Value::Null)
}


fn main() {

    let cmd_def = CliCommand::new(
        ApiMethod::new(
            backup_file,
            ObjectSchema::new("Create backup from file.")
                .required("filename", StringSchema::new("Source file name."))
                .required("store", StringSchema::new("Datastore name."))
        ))
        .arg_param(vec!["filename"]);

    if let Err(err) = run_cli_command(&cmd_def.into()) {
        eprintln!("Error: {}", err);
        print_cli_usage();
        std::process::exit(-1);
    }

}
