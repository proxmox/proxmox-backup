use std::fs::File;
use std::io::Read;
use std::collections::HashMap;
use serde_json::{json, Value};

pub struct SectionConfigPlugin {
    type_name: String,
}


pub struct SectionConfig {
    plugins: HashMap<String, SectionConfigPlugin>,

    parse_section_header: fn(String) -> Value,
}

impl SectionConfig {

    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            parse_section_header: SectionConfig::default_parse_section_header,
        }
    }

    pub fn parse(&self, filename: &str, raw: &str) {

        for line in raw.lines() {
            println!("LINE:{}", line);
        }
    }

    fn default_parse_section_header(line: String) -> Value {

        let config = json!({});

        config
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
