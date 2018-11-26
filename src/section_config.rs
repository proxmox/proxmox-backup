use failure::*;

use std::fs::File;
use std::io::Read;
use std::collections::HashMap;
use serde_json::{json, Value};

pub struct SectionConfigPlugin {
    type_name: String,
}


pub struct SectionConfig {
    plugins: HashMap<String, SectionConfigPlugin>,

    parse_section_header: fn(&str) ->  Option<(String, String)>,
}

enum ParseState {
    BeforeHeader,
    InsideSection,
}

impl SectionConfig {

    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            parse_section_header: SectionConfig::default_parse_section_header,
        }
    }

    pub fn parse(&self, filename: &str, raw: &str) -> Result<(), Error> {

        let mut line_no = 0;

	//let error_prefix = format!("file '{}' line {}", filename, line_no);
	const ERROR_FORMAT: &str = "file '{}' line {} - {}";

        let mut state = ParseState::BeforeHeader;

        for line in raw.lines() {
            line_no += 1;

            if line.trim().is_empty() { continue; }

            match state {

                ParseState::BeforeHeader => {

                    if line.trim().is_empty() { continue; }

                    if let Some((section_type, section_id)) = (self.parse_section_header)(line) {
                        println!("OKLINE: type: {} ID: {}", section_type, section_id);
                        state = ParseState::InsideSection;
                    } else {
                        println!("file '{}' line {} - {}", filename, line_no, "syntax error  - expected header");
                    }
                }
                ParseState::InsideSection => {

                    if line.trim().is_empty() {
                        // finish section
                        state = ParseState::BeforeHeader;
                        continue;
                    }
                    println!("CONTENT: {}", line);
                }
            }
        }

        if let ParseState::InsideSection = state {
            // finish section
        }

        Ok(())
    }

    pub fn default_parse_section_header(line: &str) -> Option<(String, String)> {

        if line.is_empty() { return None; };

        let first_char = line.chars().next().unwrap();

        if !first_char.is_alphabetic() { return None }

        let mut head_iter = line.splitn(2, ':');

        let section_type = match head_iter.next() {
            Some(v) => v,
            None => return None,
        };

        let section_type = section_type.trim();

        if section_type.len() == 0 { return None; }

        // fixme: verify format

        let section_id = match head_iter.next() {
            Some(v) => v,
            None => return None,
        };

        let section_id = section_id.trim();


        Some((section_type.into(), section_id.into()))
    }


}


// cargo test test_section_config1 -- --nocapture
#[test]
fn test_section_config1() {

    let filename = "storage.cfg";

    //let mut file = File::open(filename).expect("file not found");
    //let mut contents = String::new();
    //file.read_to_string(&mut contents).unwrap();

    let config = SectionConfig::new();


    let raw = r"

lvmthin: local-lvm
        thinpool data
        vgname pve5
        content rootdir,images
";

    config.parse(filename, &raw);


}
