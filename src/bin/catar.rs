extern crate proxmox_backup;

use failure::*;

use proxmox_backup::cli::command::*;
use proxmox_backup::api::schema::*;
use proxmox_backup::api::router::*;

use serde_json::{Value};

use std::io::Read;
use std::path::PathBuf;

use proxmox_backup::catar::format_definition::*;
use proxmox_backup::catar::decoder::*;

use proxmox_backup::tools::*;

fn required_string_param<'a>(param: &'a Value, name: &str) -> &'a str {
    param[name].as_str().expect(&format!("missing parameter '{}'", name))
}

fn print_goodby_entries(buffer: &[u8]) -> Result<(), Error> {
    println!("GOODBY START: {}", buffer.len());

    let entry_size = std::mem::size_of::<CaFormatGoodbyeItem>();
    if (buffer.len() % entry_size) != 0 {
        bail!("unexpected goodby item size ({})", entry_size);
    }

    let mut pos = 0;

    while pos < buffer.len() {

        let item = map_struct::<CaFormatGoodbyeItem>(&buffer[pos..pos+entry_size])?;

        if item.hash == CA_FORMAT_GOODBYE_TAIL_MARKER {
            println!("  Entry Offset: {}", item.offset);
            if item.size != (buffer.len() + 16) as u64 {
                bail!("gut unexpected goodby entry size (tail marker)");
            }
        } else {
            println!("  Offset: {}", item.offset);
            println!("  Size: {}", item.size);
            println!("  Hash: {:016x}", item.hash);
        }

        pos += entry_size;
    }

    Ok(())
}

fn print_filenames(param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    let archive = required_string_param(&param, "archive");
    let file = std::fs::File::open(archive)?;

    let mut reader = std::io::BufReader::new(file);

    let mut decoder = CaTarDecoder::new(&mut reader)?;

    let root = decoder.root();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    decoder.print_filenames(&mut out, &mut PathBuf::from("."), &root)?;

    Ok(Value::Null)
}

fn dump_archive(param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    let archive = required_string_param(&param, "archive");
    let mut file = std::fs::File::open(archive)?;

    println!("CATAR {}", archive);

    let mut buffer = [0u8; 16];

    let mut nesting = 0;

    loop {
        file.read_exact(&mut buffer)?;

        let header = map_struct::<CaFormatHeader>(&mut buffer)?;

        println!("Type: {:016x}", header.htype);
        println!("Size: {}", header.size);

        let mut rest = vec![0u8; (header.size as usize) - 16];
        file.read_exact(&mut rest)?;

        if header.htype == CA_FORMAT_FILENAME {
            let name = read_os_string(&rest);
            let hash = compute_goodbye_hash(&rest[..rest.len()-1]);
            println!("Name: {:?} {:016x}",  name, hash);
        }

        if header.htype == CA_FORMAT_ENTRY {
            let entry = map_struct::<CaFormatEntry>(&mut rest)?;
            println!("Mode: {:08x} {:08x}", entry.mode, (entry.mode as u32) & libc::S_IFDIR);
            if ((entry.mode as u32) & libc::S_IFMT) == libc::S_IFDIR {
                nesting += 1;
            }
        }
        if header.htype == CA_FORMAT_GOODBYE {
            nesting -= 1;
            print_goodby_entries(&rest)?;
            if nesting == 0 { break; }
        }
    }

    Ok(Value::Null)
}

fn create_backup(param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    let _archive = required_string_param(&param, "archive");
    let _source = required_string_param(&param, "source");


    Ok(Value::Null)
}

fn main() {

    let cmd_def = CliCommandMap::new()
        .insert("create", CliCommand::new(
            ApiMethod::new(
                create_backup,
                ObjectSchema::new("Create new catar archive.")
                    .required("archive", StringSchema::new("Archive name"))
                    .required("source", StringSchema::new("Source directory."))
            ))
            .arg_param(vec!["archive", "source"])
            .into()
        )
        .insert("list", CliCommand::new(
            ApiMethod::new(
                print_filenames,
                ObjectSchema::new("List the contents of an archive.")
                    .required("archive", StringSchema::new("Archive name."))
            ))
            .arg_param(vec!["archive"])
            .into()
        )
        .insert("dump", CliCommand::new(
            ApiMethod::new(
                dump_archive,
                ObjectSchema::new("Textual dump of archive contents (debug toolkit).")
                    .required("archive", StringSchema::new("Archive name."))
            ))
            .arg_param(vec!["archive"])
            .into()
        );

    if let Err(err) = run_cli_command(&cmd_def.into()) {
        eprintln!("Error: {}", err);
        print_cli_usage();
        std::process::exit(-1);
    }

}
