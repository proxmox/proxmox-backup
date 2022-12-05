use anyhow::{bail, Error};

use pbs_tools::nom::{multispace0, multispace1, notspace1, IResult};

use nom::{
    bytes::complete::{take_till, take_till1, take_while1},
    character::complete::{char, digit1, line_ending},
    combinator::{all_consuming, map_res, opt, recognize},
    multi::many0,
    sequence::{preceded, tuple},
};

#[derive(Debug, PartialEq)]
pub struct ZFSPoolUsage {
    pub size: u64,
    pub alloc: u64,
    pub free: u64,
    pub dedup: f64,
    pub frag: u64,
}

#[derive(Debug, PartialEq)]
pub struct ZFSPoolInfo {
    pub name: String,
    pub health: String,
    pub usage: Option<ZFSPoolUsage>,
    pub devices: Vec<String>,
}

fn parse_optional_u64(i: &str) -> IResult<&str, Option<u64>> {
    if let Some(rest) = i.strip_prefix('-') {
        Ok((rest, None))
    } else {
        let (i, value) = map_res(recognize(digit1), str::parse)(i)?;
        Ok((i, Some(value)))
    }
}

fn parse_optional_f64(i: &str) -> IResult<&str, Option<f64>> {
    if let Some(rest) = i.strip_prefix('-') {
        Ok((rest, None))
    } else {
        let (i, value) = nom::number::complete::double(i)?;
        Ok((i, Some(value)))
    }
}

fn parse_pool_device(i: &str) -> IResult<&str, String> {
    let (i, (device, _, _rest)) = tuple((
        preceded(multispace1, take_till1(|c| c == ' ' || c == '\t')),
        multispace1,
        preceded(take_till(|c| c == '\n'), char('\n')),
    ))(i)?;

    Ok((i, device.to_string()))
}

fn parse_zpool_list_header(i: &str) -> IResult<&str, ZFSPoolInfo> {
    // name, size, allocated, free, checkpoint, expandsize, fragmentation, capacity, dedupratio, health, altroot.

    let (i, (text, size, alloc, free, _, _, frag, _, dedup, health, _altroot, _eol)) = tuple((
        take_while1(|c| char::is_alphanumeric(c) || c == '-' || c == ':' || c == '_' || c == '.'), // name
        preceded(multispace1, parse_optional_u64), // size
        preceded(multispace1, parse_optional_u64), // allocated
        preceded(multispace1, parse_optional_u64), // free
        preceded(multispace1, notspace1),          // checkpoint
        preceded(multispace1, notspace1),          // expandsize
        preceded(multispace1, parse_optional_u64), // fragmentation
        preceded(multispace1, notspace1),          // capacity
        preceded(multispace1, parse_optional_f64), // dedup
        preceded(multispace1, notspace1),          // health
        opt(preceded(multispace1, notspace1)),     // optional altroot
        line_ending,
    ))(i)?;

    let status = if let (Some(size), Some(alloc), Some(free), Some(frag), Some(dedup)) =
        (size, alloc, free, frag, dedup)
    {
        ZFSPoolInfo {
            name: text.into(),
            health: health.into(),
            usage: Some(ZFSPoolUsage {
                size,
                alloc,
                free,
                frag,
                dedup,
            }),
            devices: Vec::new(),
        }
    } else {
        ZFSPoolInfo {
            name: text.into(),
            health: health.into(),
            usage: None,
            devices: Vec::new(),
        }
    };

    Ok((i, status))
}

fn parse_zpool_list_item(i: &str) -> IResult<&str, ZFSPoolInfo> {
    let (i, mut stat) = parse_zpool_list_header(i)?;
    let (i, devices) = many0(parse_pool_device)(i)?;

    for device_path in devices.into_iter().filter(|n| n.starts_with("/dev/")) {
        stat.devices.push(device_path);
    }

    let (i, _) = many0(tuple((multispace0, char('\n'))))(i)?; // skip empty lines

    Ok((i, stat))
}

/// Parse zpool list output
///
/// Note: This does not reveal any details on how the pool uses the devices, because
/// the zpool list output format is not really defined...
fn parse_zpool_list(i: &str) -> Result<Vec<ZFSPoolInfo>, Error> {
    match all_consuming(many0(parse_zpool_list_item))(i) {
        Err(nom::Err::Error(err)) | Err(nom::Err::Failure(err)) => {
            bail!(
                "unable to parse zfs list output - {}",
                nom::error::convert_error(i, err)
            );
        }
        Err(err) => {
            bail!("unable to parse zfs list output - {}", err);
        }
        Ok((_, ce)) => Ok(ce),
    }
}

/// Run zpool list and return parsed output
///
/// Devices are only included when run with verbose flags
/// set. Without, device lists are empty.
pub fn zpool_list(pool: Option<String>, verbose: bool) -> Result<Vec<ZFSPoolInfo>, Error> {
    // Note: zpools list verbose output can include entries for 'special', 'cache' and 'logs'
    // and maybe other things.

    let mut command = std::process::Command::new("zpool");
    command.args(["list", "-H", "-p", "-P"]);

    // Note: We do not use -o to define output properties, because zpool command ignores
    // that completely for special vdevs and devices

    if verbose {
        command.arg("-v");
    }

    if let Some(pool) = pool {
        command.arg(pool);
    }

    let output = proxmox_sys::command::run_command(command, None)?;

    parse_zpool_list(&output)
}

#[test]
fn test_zfs_parse_list() -> Result<(), Error> {
    let output = "";

    let data = parse_zpool_list(output)?;
    let expect = Vec::new();

    assert_eq!(data, expect);

    let output = "btest	427349245952	405504	427348840448	-	-	0	0	1.00	ONLINE	-\n";
    let data = parse_zpool_list(output)?;
    let expect = vec![ZFSPoolInfo {
        name: "btest".to_string(),
        health: "ONLINE".to_string(),
        devices: Vec::new(),
        usage: Some(ZFSPoolUsage {
            size: 427349245952,
            alloc: 405504,
            free: 427348840448,
            dedup: 1.0,
            frag: 0,
        }),
    }];

    assert_eq!(data, expect);

    let output = "\
rpool	535260299264      402852388864      132407910400      -          -          22         75         1.00      ONLINE   -
            /dev/disk/by-id/ata-Crucial_CT500MX200SSD1_154210EB4078-part3    498216206336      392175546368      106040659968      -          -          22         78         -          ONLINE
special                                                                                             -         -         -            -             -         -         -         -   -
            /dev/sda2          37044092928       10676842496       26367250432       -          -          63         28         -          ONLINE
logs                                                                                                 -         -         -            -             -         -         -         -   -
            /dev/sda3          4831838208         1445888 4830392320         -          -          0          0          -          ONLINE

";

    let data = parse_zpool_list(output)?;
    let expect = vec![
        ZFSPoolInfo {
            name: String::from("rpool"),
            health: String::from("ONLINE"),
            devices: vec![String::from(
                "/dev/disk/by-id/ata-Crucial_CT500MX200SSD1_154210EB4078-part3",
            )],
            usage: Some(ZFSPoolUsage {
                size: 535260299264,
                alloc: 402852388864,
                free: 132407910400,
                dedup: 1.0,
                frag: 22,
            }),
        },
        ZFSPoolInfo {
            name: String::from("special"),
            health: String::from("-"),
            devices: vec![String::from("/dev/sda2")],
            usage: None,
        },
        ZFSPoolInfo {
            name: String::from("logs"),
            health: String::from("-"),
            devices: vec![String::from("/dev/sda3")],
            usage: None,
        },
    ];

    assert_eq!(data, expect);

    let output = "\
b-test	427349245952	761856	427348484096	-	-	0	0	1.00	ONLINE	-
	mirror	213674622976	438272	213674184704	-	-	0	0	-	ONLINE
	/dev/sda1	-	-	-	-	-	-	-	-	ONLINE
	/dev/sda2	-	-	-	-	-	-	-	-	ONLINE
	mirror	213674622976	323584	213674299392	-	-	0	0	-	ONLINE
	/dev/sda3	-	-	-	-	-	-	-	-	ONLINE
	/dev/sda4	-	-	-	-	-	-	-	-	ONLINE
logs               -      -      -        -         -      -      -      -  -
	/dev/sda5	213674622976	0	213674622976	-	-	0	0	-	ONLINE
";

    let data = parse_zpool_list(output)?;
    let expect = vec![
        ZFSPoolInfo {
            name: String::from("b-test"),
            health: String::from("ONLINE"),
            usage: Some(ZFSPoolUsage {
                size: 427349245952,
                alloc: 761856,
                free: 427348484096,
                dedup: 1.0,
                frag: 0,
            }),
            devices: vec![
                String::from("/dev/sda1"),
                String::from("/dev/sda2"),
                String::from("/dev/sda3"),
                String::from("/dev/sda4"),
            ],
        },
        ZFSPoolInfo {
            name: String::from("logs"),
            health: String::from("-"),
            usage: None,
            devices: vec![String::from("/dev/sda5")],
        },
    ];

    assert_eq!(data, expect);

    let output = "\
b.test	427349245952	761856	427348484096	-	-	0	0	1.00	ONLINE	-
	mirror	213674622976	438272	213674184704	-	-	0	0	-	ONLINE
	/dev/sda1	-	-	-	-	-	-	-	-	ONLINE
";

    let data = parse_zpool_list(output)?;
    let expect = vec![ZFSPoolInfo {
        name: String::from("b.test"),
        health: String::from("ONLINE"),
        usage: Some(ZFSPoolUsage {
            size: 427349245952,
            alloc: 761856,
            free: 427348484096,
            dedup: 1.0,
            frag: 0,
        }),
        devices: vec![String::from("/dev/sda1")],
    }];

    assert_eq!(data, expect);

    Ok(())
}
