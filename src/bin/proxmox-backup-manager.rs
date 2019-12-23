use failure::*;
use serde_json::{json, Value};
use std::path::PathBuf;
use nix::sys::stat::Mode;

use proxmox::tools::fs::{CreateOptions, replace_file};
use proxmox::api::{api, cli::*};

use proxmox_backup::configdir;
use proxmox_backup::tools;
use proxmox_backup::config;
use proxmox_backup::backup::*;
use proxmox_backup::api2::types::*;
use proxmox_backup::client::*;
use proxmox_backup::tools::ticket::*;
use proxmox_backup::auth_helpers::*;


async fn view_task_result(
    client: HttpClient,
    result: Value,
    output_format: &str,
) -> Result<(), Error> {
    let data = &result["data"];
    if output_format == "text" {
        if let Some(upid) = data.as_str() {
            display_task_log(client, upid, true).await?;
        }
    } else {
        format_and_print_result(&data, &output_format);
    }

    Ok(())
}

fn connect() -> Result<HttpClient, Error> {

    let uid = nix::unistd::Uid::current();

    let client = if uid.is_root()  {
        let ticket = assemble_rsa_ticket(private_auth_key(), "PBS", Some("root@pam"), None)?;
        HttpClient::new("localhost", "root@pam", Some(ticket))?
    } else {
        HttpClient::new("localhost", "root@pam", None)?
    };

    Ok(client)
}

fn datastore_commands() -> CommandLineInterface {

    use proxmox_backup::api2;

    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&api2::config::datastore::GET))
        .insert("create",
                CliCommand::new(&api2::config::datastore::POST)
                .arg_param(&["name", "path"])
        )
        .insert("remove",
                CliCommand::new(&api2::config::datastore::DELETE)
                .arg_param(&["name"])
                .completion_cb("name", config::datastore::complete_datastore_name)
        );

    cmd_def.into()
}


#[api(
   input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
   }
)]
/// Start garbage collection for a specific datastore.
async fn start_garbage_collection(param: Value) -> Result<Value, Error> {

    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    let store = tools::required_string_param(&param, "store")?;

    let mut client = connect()?;

    let path = format!("api2/json/admin/datastore/{}/gc", store);

    let result = client.post(&path, None).await?;

    view_task_result(client, result, &output_format).await?;

    Ok(Value::Null)
}

#[api(
   input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
   }
)]
/// Show garbage collection status for a specific datastore.
async fn garbage_collection_status(param: Value) -> Result<Value, Error> {

    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    let store = tools::required_string_param(&param, "store")?;

    let client = connect()?;

    let path = format!("api2/json/admin/datastore/{}/gc", store);

    let result = client.get(&path, None).await?;
    let data = &result["data"];
    if output_format == "text" {
        format_and_print_result(&data, "json-pretty");
     } else {
        format_and_print_result(&data, &output_format);
    }

    Ok(Value::Null)
}

fn garbage_collection_commands() -> CommandLineInterface {

    let cmd_def = CliCommandMap::new()
        .insert("status",
                CliCommand::new(&API_METHOD_GARBAGE_COLLECTION_STATUS)
                .arg_param(&["store"])
                .completion_cb("store", config::datastore::complete_datastore_name)
        )
        .insert("start",
                CliCommand::new(&API_METHOD_START_GARBAGE_COLLECTION)
                .arg_param(&["store"])
                .completion_cb("store", config::datastore::complete_datastore_name)
        );

    cmd_def.into()
}

#[api(
    input: {
        properties: {
            limit: {
                description: "The maximal number of tasks to list.",
                type: Integer,
                optional: true,
                minimum: 1,
                maximum: 1000,
                default: 50,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
            all: {
                type: Boolean,
                description: "Also list stopped tasks.",
                optional: true,
            }
        }
    }
)]
/// List running server tasks.
async fn task_list(param: Value) -> Result<Value, Error> {

    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    let client = connect()?;

    let limit = param["limit"].as_u64().unwrap_or(50) as usize;
    let running = !param["all"].as_bool().unwrap_or(false);
    let args = json!({
        "running": running,
        "start": 0,
        "limit": limit,
    });
    let result = client.get("api2/json/nodes/localhost/tasks", Some(args)).await?;

    let data = &result["data"];

    if output_format == "text" {
        for item in data.as_array().unwrap() {
            println!(
                "{} {}",
                item["upid"].as_str().unwrap(),
                item["status"].as_str().unwrap_or("running"),
            );
        }
    } else {
        format_and_print_result(data, &output_format);
    }

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            upid: {
                schema: UPID_SCHEMA,
            },
        }
    }
)]
/// Display the task log.
async fn task_log(param: Value) -> Result<Value, Error> {

    let upid = tools::required_string_param(&param, "upid")?;

    let client = connect()?;

    display_task_log(client, upid, true).await?;

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            upid: {
                schema: UPID_SCHEMA,
            },
        }
    }
)]
/// Try to stop a specific task.
async fn task_stop(param: Value) -> Result<Value, Error> {

    let upid_str = tools::required_string_param(&param, "upid")?;

    let mut client = connect()?;

    let path = format!("api2/json/nodes/localhost/tasks/{}", upid_str);
    let _ = client.delete(&path, None).await?;

    Ok(Value::Null)
}

fn task_mgmt_cli() -> CommandLineInterface {

    let task_log_cmd_def = CliCommand::new(&API_METHOD_TASK_LOG)
        .arg_param(&["upid"]);

    let task_stop_cmd_def = CliCommand::new(&API_METHOD_TASK_STOP)
        .arg_param(&["upid"]);

    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_TASK_LIST))
        .insert("log", task_log_cmd_def)
        .insert("stop", task_stop_cmd_def);

    cmd_def.into()
}

fn x509name_to_string(name: &openssl::x509::X509NameRef) -> Result<String, Error> {
    let mut parts = Vec::new();
    for entry in name.entries() {
        parts.push(format!("{} = {}", entry.object().nid().short_name()?, entry.data().as_utf8()?));
    }
    Ok(parts.join(", "))
}

#[api]
/// Diplay node certificate information.
fn cert_info() -> Result<(), Error> {

    let cert_path = PathBuf::from(configdir!("/proxy.pem"));

    let cert_pem = proxmox::tools::fs::file_get_contents(&cert_path)?;

    let cert = openssl::x509::X509::from_pem(&cert_pem)?;

    println!("Subject: {}", x509name_to_string(cert.subject_name())?);

    if let Some(san) = cert.subject_alt_names() {
        for name in san.iter() {
            if let Some(v) = name.dnsname() {
                println!("    DNS:{}", v);
            } else if let Some(v) = name.ipaddress() {
                println!("    IP:{:?}", v);
            } else if let Some(v) = name.email() {
                println!("    EMAIL:{}", v);
            } else if let Some(v) = name.uri() {
                println!("    URI:{}", v);
            }
        }
    }

    println!("Issuer: {}", x509name_to_string(cert.issuer_name())?);
    println!("Validity:");
    println!("    Not Before: {}", cert.not_before());
    println!("    Not After : {}", cert.not_after());

    let fp = cert.digest(openssl::hash::MessageDigest::sha256())?;
    let fp_string = proxmox::tools::digest_to_hex(&fp);
    let fp_string = fp_string.as_bytes().chunks(2).map(|v| std::str::from_utf8(v).unwrap())
        .collect::<Vec<&str>>().join(":");

    println!("Fingerprint (sha256): {}", fp_string);

    let pubkey = cert.public_key()?;
    println!("Public key type: {}", openssl::nid::Nid::from_raw(pubkey.id().as_raw()).long_name()?);
    println!("Public key bits: {}", pubkey.bits());

    Ok(())
}

#[api(
    input: {
        properties: {
            force: {
	        description: "Force generation of new SSL certifate.",
	        type:  Boolean,
	        optional:true,
	    },
        }
    },
)]
/// Update node certificates and generate all needed files/directories.
fn update_certs(force: Option<bool>) -> Result<(), Error> {

    let backup_user = backup_user()?;

    config::create_configdir()?;

    if let Err(err) = generate_auth_key() {
        bail!("unable to generate auth key - {}", err);
    }

    if let Err(err) = generate_csrf_key() {
        bail!("unable to generate csrf key - {}", err);
    }

    //openssl req -x509 -newkey rsa:4096 -keyout /etc/proxmox-backup/proxy.key -out /etc/proxmox-backup/proxy.pem -nodes
    let key_path = PathBuf::from(configdir!("/proxy.key"));
    let cert_path = PathBuf::from(configdir!("/proxy.pem"));

    if key_path.exists() && cert_path.exists() && !force.unwrap_or(false) { return Ok(()); }

    use openssl::rsa::{Rsa};
    use openssl::x509::{X509Builder};
    use openssl::pkey::PKey;

    let rsa = Rsa::generate(4096).unwrap();

    let priv_pem = rsa.private_key_to_pem()?;

    replace_file(
        &key_path,
        &priv_pem,
        CreateOptions::new()
            .perm(Mode::from_bits_truncate(0o0640))
            .owner(nix::unistd::ROOT)
            .group(backup_user.gid),
    )?;

    let mut x509 = X509Builder::new()?;

    x509.set_version(2)?;

    let today = openssl::asn1::Asn1Time::days_from_now(0)?;
    x509.set_not_before(&today)?;
    let expire = openssl::asn1::Asn1Time::days_from_now(365*1000)?;
    x509.set_not_after(&expire)?;

    let nodename = proxmox::tools::nodename();
    let mut fqdn = nodename.to_owned();

    let resolv_conf = proxmox_backup::api2::node::dns::read_etc_resolv_conf()?;
    if let Some(search) = resolv_conf["search"].as_str() {
        fqdn.push('.');
        fqdn.push_str(search);
    }

    // we try to generate an unique 'subject' to avoid browser problems
    //(reused serial numbers, ..)
    let uuid = proxmox::tools::uuid::Uuid::generate();

    let mut subject_name = openssl::x509::X509NameBuilder::new()?;
    subject_name.append_entry_by_text("O", "Proxmox Backup Server")?;
    subject_name.append_entry_by_text("OU", &format!("{:X}", uuid))?;
    subject_name.append_entry_by_text("CN", &fqdn)?;
    let subject_name = subject_name.build();

    x509.set_subject_name(&subject_name)?;
    x509.set_issuer_name(&subject_name)?;

    let bc = openssl::x509::extension::BasicConstraints::new(); // CA = false
    let bc = bc.build()?;
    x509.append_extension(bc)?;

    let usage = openssl::x509::extension::ExtendedKeyUsage::new()
        .server_auth()
        .build()?;
    x509.append_extension(usage)?;

    let context = x509.x509v3_context(None, None);

    let mut alt_names = openssl::x509::extension::SubjectAlternativeName::new();

    alt_names.ip("127.0.0.1");
    alt_names.ip("::1");

    // fixme: add local node IPs

    alt_names.dns("localhost");

    if nodename != "localhost" { alt_names.dns(nodename); }
    if nodename != fqdn { alt_names.dns(&fqdn); }

    let alt_names = alt_names.build(&context)?;

    x509.append_extension(alt_names)?;

    let pub_pem = rsa.public_key_to_pem()?;
    let pubkey = PKey::public_key_from_pem(&pub_pem)?;

    x509.set_pubkey(&pubkey)?;

    let context = x509.x509v3_context(None, None);
    let ext = openssl::x509::extension::SubjectKeyIdentifier::new().build(&context)?;
    x509.append_extension(ext)?;

    let context = x509.x509v3_context(None, None);
    let ext = openssl::x509::extension::AuthorityKeyIdentifier::new()
        .keyid(true)
        .build(&context)?;
    x509.append_extension(ext)?;

    let privkey = PKey::from_rsa(rsa)?;

    x509.sign(&privkey, openssl::hash::MessageDigest::sha256())?;

    let x509 = x509.build();
    let cert_pem = x509.to_pem()?;

    replace_file(
        &cert_path,
        &cert_pem,
        CreateOptions::new()
            .perm(Mode::from_bits_truncate(0o0640))
            .owner(nix::unistd::ROOT)
            .group(backup_user.gid),
    )?;

    Ok(())
}

fn cert_mgmt_cli() -> CommandLineInterface {

    let cmd_def = CliCommandMap::new()
        .insert("info", CliCommand::new(&API_METHOD_CERT_INFO))
        .insert("update", CliCommand::new(&API_METHOD_UPDATE_CERTS));

    cmd_def.into()
}

fn main() {

    let cmd_def = CliCommandMap::new()
        .insert("datastore", datastore_commands())
        .insert("garbage-collection", garbage_collection_commands())
        .insert("cert", cert_mgmt_cli())
        .insert("task", task_mgmt_cli());

    run_cli_command(cmd_def);
}
