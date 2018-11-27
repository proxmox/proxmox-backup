use failure::*;

use std::fs::File;
use std::io::Read;
use std::collections::HashMap;
use serde_json::{json, Value};

use std::sync::Arc;
use crate::api::schema::*;

pub struct SectionConfigPlugin {
    type_name: String,
    properties: ObjectSchema,
}

impl SectionConfigPlugin {

    pub fn new(type_name: String, properties: ObjectSchema) -> Self {
        Self { type_name, properties }
    }

}

pub struct SectionConfig {
    plugins: HashMap<String, SectionConfigPlugin>,

    parse_section_header: fn(&str) ->  Option<(String, String)>,
    parse_section_content: fn(&str) ->  Option<(String, String)>,
}

enum ParseState<'a> {
    BeforeHeader,
    InsideSection(&'a SectionConfigPlugin, Value),
}

impl SectionConfig {

    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            parse_section_header: SectionConfig::default_parse_section_header,
            parse_section_content: SectionConfig::default_parse_section_content,
        }
    }

    pub fn register_plugin(&mut self, plugin: SectionConfigPlugin) {
        self.plugins.insert(plugin.type_name.clone(), plugin);
    }

    pub fn parse(&self, filename: &str, raw: &str) -> Result<(), Error> {

        let mut line_no = 0;

        let mut state = ParseState::BeforeHeader;

        let test_required_properties = |value: &Value, schema: &ObjectSchema| -> Result<(), Error> {
            for (name, (optional, _prop_schema)) in &schema.properties {
                if *optional == false && value[name] == Value::Null {
                    return Err(format_err!("property '{}' is missing and it is not optional.", name));
                }
            }
            Ok(())
        };

        for line in raw.lines() {
            line_no += 1;

            if line.trim().is_empty() { continue; }

            match state {

                ParseState::BeforeHeader => {

                    if line.trim().is_empty() { continue; }

                    if let Some((section_type, section_id)) = (self.parse_section_header)(line) {
                        println!("OKLINE: type: {} ID: {}", section_type, section_id);
                        if let Some(ref plugin) = self.plugins.get(&section_type) {
                            state = ParseState::InsideSection(plugin, json!({}));
                        } else {
                            bail!("file '{}' line {} - unknown section type '{}'",
                                  filename, line_no, section_type);
                       }
                    } else {
                        bail!("file '{}' line {} - syntax error (expected header)", filename, line_no);
                    }
                }
                ParseState::InsideSection(plugin, ref mut config) => {

                    if line.trim().is_empty() {
                        // finish section
                        if let Err(err) = test_required_properties(config, &plugin.properties) {
                            bail!("file '{}' line {} - {}", filename, line_no, err.to_string());
                        }
                        state = ParseState::BeforeHeader;
                        continue;
                    }
                    println!("CONTENT: {}", line);
                    if let Some((key, value)) = (self.parse_section_content)(line) {
                        println!("CONTENT: key: {} value: {}", key, value);

                        if let Some((_optional, prop_schema)) = plugin.properties.properties.get::<str>(&key) {
                            match parse_simple_value(&value, prop_schema) {
                                Ok(value) => {
                                    if config[&key] == Value::Null {
                                        config[key] = value;
                                    } else {
                                        bail!("file '{}' line {} - duplicate property '{}'",
                                              filename, line_no, key);
                                    }
                                }
                                Err(err) => {
                                    bail!("file '{}' line {} - property '{}': {}",
                                          filename, line_no, key, err.to_string());
                                }
                            }
                        } else {
                            bail!("file '{}' line {} - unknown property '{}'", filename, line_no, key)
                        }
                    } else {
                        bail!("file '{}' line {} - syntax error (expected section properties)", filename, line_no);
                    }
                }
            }
        }

        if let ParseState::InsideSection(plugin, ref config) = state {
            // finish section
            if let Err(err) = test_required_properties(config, &plugin.properties) {
                bail!("file '{}' line {} - {}", filename, line_no, err.to_string());
            }
        }

        Ok(())
    }

    pub fn default_parse_section_content(line: &str) -> Option<(String, String)> {

        if line.is_empty() { return None; }
        let first_char = line.chars().next().unwrap();

        if !first_char.is_whitespace() { return None }

        let mut kv_iter = line.trim_left().splitn(2, |c: char| c.is_whitespace());

        let key = match kv_iter.next() {
            Some(v) => v.trim(),
            None => return None,
        };

        if key.len() == 0 { return None; }

        let value = match kv_iter.next() {
            Some(v) => v.trim(),
            None => return None,
        };

        Some((key.into(), value.into()))
   }

    pub fn default_parse_section_header(line: &str) -> Option<(String, String)> {

        if line.is_empty() { return None; };

        let first_char = line.chars().next().unwrap();

        if !first_char.is_alphabetic() { return None }

        let mut head_iter = line.splitn(2, ':');

        let section_type = match head_iter.next() {
            Some(v) => v.trim(),
            None => return None,
        };

        if section_type.len() == 0 { return None; }

        // fixme: verify format

        let section_id = match head_iter.next() {
            Some(v) => v.trim(),
            None => return None,
        };

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

    let plugin = SectionConfigPlugin::new(
        "lvmthin".to_string(),
        ObjectSchema::new("lvmthin properties")
            .required("thinpool", StringSchema::new("LVM thin pool name."))
            .required("vgname", StringSchema::new("LVM volume group name."))
            .optional("content", StringSchema::new("Storage content types."))
    );

    let mut config = SectionConfig::new();
    config.register_plugin(plugin);

    let raw = r"

lvmthin: local-lvm
        thinpool data
        vgname pve5
        content rootdir,images
";

    let res = config.parse(filename, &raw);
    println!("RES: {:?}", res);

}
