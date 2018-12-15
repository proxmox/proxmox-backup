extern crate apitest;

use failure::*;

use apitest::tools;
use apitest::cli::command::*;
use apitest::api::schema::*;
use apitest::api::router::*;
use apitest::backup::chunk_store::*;
use serde_json::{Value};

use apitest::config::datastore;

fn required_string_param<'a>(param: &'a Value, name: &str) -> &'a str {
    param[name].as_str().expect(&format!("missing parameter '{}'", name))
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

    tools::file_chunker(file, 64*1024, |pos, chunk| {
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
