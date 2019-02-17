use failure::*;

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;

use serde_json::{json, Value};

use std::sync::Arc;
use crate::api_schema::schema::*;

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

    id_schema: Arc<Schema>,
    parse_section_header: fn(&str) -> Option<(String, String)>,
    parse_section_content: fn(&str) -> Option<(String, String)>,
    format_section_header: fn(type_name: &str, section_id: &str, data: &Value) -> String,
}

enum ParseState<'a> {
    BeforeHeader,
    InsideSection(&'a SectionConfigPlugin, String, Value),
}

#[derive(Debug)]
pub struct SectionConfigData {
    pub sections: HashMap<String, (String, Value)>,
    order: VecDeque<String>,
}

impl SectionConfigData {

    pub fn new() -> Self {
        Self { sections: HashMap::new(), order: VecDeque::new() }
    }

    pub fn set_data(&mut self, section_id: &str, type_name: &str, config: Value) {
        // fixme: verify section_id schema here??
        self.sections.insert(section_id.to_string(), (type_name.to_string(), config));
    }

    fn record_order(&mut self, section_id: &str) {
        self.order.push_back(section_id.to_string());
    }

    pub fn convert_to_array(&self, id_prop: &str) -> Value {
        let mut list: Vec<Value> = vec![];

        for (section_id, (_, data)) in &self.sections {
            let mut item = data.clone();
            item.as_object_mut().unwrap().insert(id_prop.into(), section_id.clone().into());
            list.push(item);
        }

        list.into()
    }
}

impl SectionConfig {

    pub fn new(id_schema: Arc<Schema>) -> Self {
        Self {
            plugins: HashMap::new(),
            id_schema: id_schema,
            parse_section_header: SectionConfig::default_parse_section_header,
            parse_section_content: SectionConfig::default_parse_section_content,
            format_section_header: SectionConfig::default_format_section_header,
        }
    }

    pub fn register_plugin(&mut self, plugin: SectionConfigPlugin) {
        self.plugins.insert(plugin.type_name.clone(), plugin);
    }

    pub fn write(&self, _filename: &str, config: &SectionConfigData) -> Result<String, Error> {

        let mut list = VecDeque::new();

        let mut done = HashSet::new();

        for section_id in &config.order {
            if config.sections.get(section_id) == None { continue };
            list.push_back(section_id);
            done.insert(section_id);
        }

        for (section_id, _) in &config.sections {
            if done.contains(section_id) { continue };
            list.push_back(section_id);
        }

        let mut raw = String::new();

        for section_id in list {
            let (type_name, section_config) = config.sections.get(section_id).unwrap();
            let plugin = self.plugins.get(type_name).unwrap();

            if let Err(err) = parse_simple_value(&section_id, &self.id_schema) {
                bail!("syntax error in section identifier: {}", err.to_string());
            }

            verify_json_object(section_config, &plugin.properties)?;
            println!("REAL WRITE {} {} {:?}\n", section_id, type_name, section_config);

            let head = (self.format_section_header)(type_name, section_id, section_config);

            if !raw.is_empty() { raw += "\n" }

            raw += &head;

            for (key, value) in section_config.as_object().unwrap() {
                let text = match value {
                    Value::Null => { continue; }, // do nothing (delete)
                    Value::Bool(v) => v.to_string(),
                    Value::String(v) => v.to_string(),
                    Value::Number(v) => v.to_string(),
                    _ => {
                        bail!("got unsupported type in section '{}' key '{}'", section_id, key);
                    },
                };
                raw += "\t";
                raw += &key;
                raw += " ";
                raw += &text;
                raw += "\n";
            }
        }

        Ok(raw)
    }

    pub fn parse(&self, filename: &str, raw: &str) -> Result<SectionConfigData, Error> {

        let mut state = ParseState::BeforeHeader;

        let test_required_properties = |value: &Value, schema: &ObjectSchema| -> Result<(), Error> {
            for (name, (optional, _prop_schema)) in &schema.properties {
                if *optional == false && value[name] == Value::Null {
                    return Err(format_err!("property '{}' is missing and it is not optional.", name));
                }
            }
            Ok(())
        };

        let mut line_no = 0;

        try_block!({

            let mut result = SectionConfigData::new();

            let mut create_section = |section_id: &str, type_name: &str, config| {
                result.set_data(section_id, type_name, config);
                result.record_order(section_id);
            };

            try_block!({
                for line in raw.lines() {
                    line_no += 1;

                    match state {

                        ParseState::BeforeHeader => {

                            if line.trim().is_empty() { continue; }

                            if let Some((section_type, section_id)) = (self.parse_section_header)(line) {
                                //println!("OKLINE: type: {} ID: {}", section_type, section_id);
                                if let Some(ref plugin) = self.plugins.get(&section_type) {
                                    if let Err(err) = parse_simple_value(&section_id, &self.id_schema) {
                                        bail!("syntax error in section identifier: {}", err.to_string());
                                    }
                                    state = ParseState::InsideSection(plugin, section_id, json!({}));
                                } else {
                                    bail!("unknown section type '{}'", section_type);
                                }
                            } else {
                                bail!("syntax error (expected header)");
                            }
                        }
                        ParseState::InsideSection(plugin, ref mut section_id, ref mut config) => {

                            if line.trim().is_empty() {
                                // finish section
                                test_required_properties(config, &plugin.properties)?;
                                create_section(section_id, &plugin.type_name, config.take());
                                state = ParseState::BeforeHeader;
                                continue;
                            }
                            if let Some((key, value)) = (self.parse_section_content)(line) {
                                //println!("CONTENT: key: {} value: {}", key, value);

                                if let Some((_optional, prop_schema)) = plugin.properties.properties.get::<str>(&key) {
                                    match parse_simple_value(&value, prop_schema) {
                                        Ok(value) => {
                                            if config[&key] == Value::Null {
                                                config[key] = value;
                                            } else {
                                                bail!("duplicate property '{}'", key);
                                            }
                                        }
                                        Err(err) => {
                                            bail!("property '{}': {}", key, err.to_string());
                                        }
                                    }
                                } else {
                                    bail!("unknown property '{}'", key)
                                }
                            } else {
                                bail!("syntax error (expected section properties)");
                            }
                        }
                    }
                }

                if let ParseState::InsideSection(plugin, section_id, config) = state {
                    // finish section
                    test_required_properties(&config, &plugin.properties)?;
                    create_section(&section_id, &plugin.type_name, config);
                }

                Ok(())

            }).map_err(|e| format_err!("line {} - {}", line_no, e))?;

            Ok(result)

        }).map_err(|e: Error| format_err!("parsing '{}' failed: {}", filename, e))
    }

    pub fn default_format_section_header(type_name: &str, section_id: &str, _data: &Value) -> String {
        return format!("{}: {}\n", type_name, section_id);
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

    let id_schema = StringSchema::new("Storage ID schema.")
        .min_length(3)
        .into();

    let mut config = SectionConfig::new(id_schema);
    config.register_plugin(plugin);

    let raw = r"

lvmthin: local-lvm
        thinpool data
        vgname pve5
        content rootdir,images

lvmthin: local-lvm2
        thinpool data
        vgname pve5
        content rootdir,images
";

    let res = config.parse(filename, &raw);
    println!("RES: {:?}", res);
    let raw = config.write(filename, &res.unwrap());
    println!("CONFIG:\n{}", raw.unwrap());


}
