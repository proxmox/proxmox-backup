use anyhow::Error;

use nom::bytes::complete::{tag, take_while};

use pbs_tape::{DriveStatus, ElementStatus, MtxStatus, StorageElementStatus};

use pbs_tools::nom::{
    multispace0, multispace1, parse_complete, parse_error, parse_failure, parse_u64, IResult,
};

// Recognizes one line
fn next_line(i: &str) -> IResult<&str, &str> {
    let (i, line) = take_while(|c| (c != '\n'))(i)?;
    if i.is_empty() {
        Ok((i, line))
    } else {
        Ok((&i[1..], line))
    }
}

fn parse_storage_changer(i: &str) -> IResult<&str, ()> {
    let (i, _) = multispace0(i)?;
    let (i, _) = tag("Storage Changer")(i)?;
    let (i, _) = next_line(i)?; // skip

    Ok((i, ()))
}

fn parse_drive_status(i: &str, id: u64) -> IResult<&str, DriveStatus> {
    let mut loaded_slot = None;

    if let Some(empty) = i.strip_prefix("Empty") {
        let status = DriveStatus {
            loaded_slot,
            status: ElementStatus::Empty,
            drive_serial_number: None,
            vendor: None,
            model: None,
            element_address: id as u16,
        };
        return Ok((empty, status));
    }
    let (mut i, _) = tag("Full (")(i)?;

    if let Some(n) = i.strip_prefix("Storage Element ") {
        let (n, id) = parse_u64(n)?;
        loaded_slot = Some(id);
        let (n, _) = tag(" Loaded")(n)?;
        i = n;
    } else {
        let (n, _) = take_while(|c| !(c == ')' || c == '\n'))(i)?; // skip to ')'
        i = n;
    }

    let (i, _) = tag(")")(i)?;

    if let Some(i) = i.strip_prefix(":VolumeTag = ") {
        let (i, tag) = take_while(|c| !(c == ' ' || c == ':' || c == '\n'))(i)?;
        let (i, _) = take_while(|c| c != '\n')(i)?; // skip to eol
        let status = DriveStatus {
            loaded_slot,
            status: ElementStatus::VolumeTag(tag.to_string()),
            drive_serial_number: None,
            vendor: None,
            model: None,
            element_address: id as u16,
        };
        return Ok((i, status));
    }

    let (i, _) = take_while(|c| c != '\n')(i)?; // skip

    let status = DriveStatus {
        loaded_slot,
        status: ElementStatus::Full,
        drive_serial_number: None,
        vendor: None,
        model: None,
        element_address: id as u16,
    };
    Ok((i, status))
}

fn parse_slot_status(i: &str) -> IResult<&str, ElementStatus> {
    if let Some(empty) = i.strip_prefix("Empty") {
        return Ok((empty, ElementStatus::Empty));
    }
    if let Some(n) = i.strip_prefix("Full ") {
        if let Some(n) = n.strip_prefix(":VolumeTag=") {
            let (n, tag) = take_while(|c| !(c == ' ' || c == ':' || c == '\n'))(n)?;
            let (n, _) = take_while(|c| c != '\n')(n)?; // skip to eol
            return Ok((n, ElementStatus::VolumeTag(tag.to_string())));
        }
        let (n, _) = take_while(|c| c != '\n')(n)?; // skip

        return Ok((n, ElementStatus::Full));
    }

    Err(parse_error(i, "unexptected element status"))
}

fn parse_data_transfer_element(i: &str) -> IResult<&str, (u64, DriveStatus)> {
    let (i, _) = tag("Data Transfer Element")(i)?;
    let (i, _) = multispace1(i)?;
    let (i, id) = parse_u64(i)?;
    let (i, _) = nom::character::complete::char(':')(i)?;
    let (i, element_status) = parse_drive_status(i, id)?;
    let (i, _) = nom::character::complete::newline(i)?;

    Ok((i, (id, element_status)))
}

fn parse_storage_element(i: &str) -> IResult<&str, (u64, bool, ElementStatus)> {
    let (i, _) = multispace1(i)?;
    let (i, _) = tag("Storage Element")(i)?;
    let (i, _) = multispace1(i)?;
    let (i, id) = parse_u64(i)?;
    let (i, opt_ie) = nom::combinator::opt(tag(" IMPORT/EXPORT"))(i)?;
    let import_export = opt_ie.is_some();
    let (i, _) = nom::character::complete::char(':')(i)?;
    let (i, element_status) = parse_slot_status(i)?;
    let (i, _) = nom::character::complete::newline(i)?;

    Ok((i, (id, import_export, element_status)))
}

fn parse_status(i: &str) -> IResult<&str, MtxStatus> {
    let (mut i, _) = parse_storage_changer(i)?;

    let mut drives = Vec::new();
    while let Ok((n, (id, drive_status))) = parse_data_transfer_element(i) {
        if id != drives.len() as u64 {
            return Err(parse_failure(i, "unexpected drive number"));
        }
        i = n;
        drives.push(drive_status);
    }

    let mut slots = Vec::new();
    while let Ok((n, (id, import_export, element_status))) = parse_storage_element(i) {
        if id != (slots.len() as u64 + 1) {
            return Err(parse_failure(i, "unexpected slot number"));
        }
        i = n;
        let status = StorageElementStatus {
            import_export,
            status: element_status,
            element_address: id as u16,
        };
        slots.push(status);
    }

    let status = MtxStatus {
        drives,
        slots,
        transports: Vec::new(),
    };

    Ok((i, status))
}

/// Parses the output from 'mtx status'
pub fn parse_mtx_status(i: &str) -> Result<MtxStatus, Error> {
    let status = parse_complete("mtx status", i, parse_status)?;

    Ok(status)
}

#[test]
fn test_changer_status() -> Result<(), Error> {
    let output = r###" Storage Changer /dev/tape/by-id/scsi-387408F60F0000:2 Drives, 24 Slots ( 4 Import/Export )
Data Transfer Element 0:Empty
Data Transfer Element 1:Empty
      Storage Element 1:Full :VolumeTag=CLN002CU
      Storage Element 2:Full :VolumeTag=test22L1
      Storage Element 3:Full :VolumeTag=test23L1
      Storage Element 4:Full :VolumeTag=CLN001CU
      Storage Element 5:Full :VolumeTag=test1
      Storage Element 6:Empty
      Storage Element 7:Empty
      Storage Element 8:Empty
      Storage Element 9:Empty
      Storage Element 10:Empty
      Storage Element 11:Empty
      Storage Element 12:Empty
      Storage Element 13:Empty
      Storage Element 14:Empty
      Storage Element 15:Empty
      Storage Element 16:Empty
      Storage Element 17:Empty
      Storage Element 18:Empty
      Storage Element 19:Empty
      Storage Element 20:Empty
      Storage Element 21 IMPORT/EXPORT:Empty
      Storage Element 22 IMPORT/EXPORT:Empty
      Storage Element 23 IMPORT/EXPORT:Empty
      Storage Element 24 IMPORT/EXPORT:Empty
"###;

    let _ = parse_mtx_status(output)?;

    Ok(())
}
