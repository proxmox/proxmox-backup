extern crate apitest;

use failure::*;

use std::collections::HashMap;

use apitest::cli::command::*;
use apitest::api::schema::*;
use apitest::api::router::*;
use apitest::backup::chunk_store::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use apitest::config::datastore;

fn backup_file(param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    println!("Backup file '{}'", param["filename"].as_str().unwrap());
    
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
