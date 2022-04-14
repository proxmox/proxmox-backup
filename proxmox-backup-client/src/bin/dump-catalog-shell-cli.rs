use anyhow::Error;

use proxmox_router::cli::*;
use proxmox_schema::format::*;

use pbs_client::catalog_shell::catalog_shell_cli;

fn main() -> Result<(), Error> {
    match catalog_shell_cli() {
        CommandLineInterface::Nested(map) => {
            let usage = generate_nested_usage("", &map, DocumentationFormat::ReST);
            println!("{}", usage);
        }
        _ => unreachable!(),
    }

    Ok(())
}
