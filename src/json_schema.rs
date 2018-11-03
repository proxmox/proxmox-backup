use std::collections::HashMap;

pub type PropertyMap = HashMap<&'static str, Jss>;

#[derive(Debug)]
pub struct JssBoolean {
    pub description: &'static str,
    pub optional: Option<bool>,
    pub default: Option<bool>,
}

#[derive(Debug)]
pub struct JssInteger {
    pub description: &'static str,
    pub optional: Option<bool>,
    pub minimum: Option<usize>,
    pub maximum: Option<usize>,
    pub default: Option<usize>,
}

#[derive(Debug)]
pub struct JssString {
    pub description: &'static str,
    pub optional: Option<bool>,
    pub default: Option<&'static str>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
}

#[derive(Debug)]
pub struct JssArray {
    pub description: &'static str,
    pub optional: Option<bool>,
    pub items: Box<Jss>,
}

#[derive(Debug)]
pub struct JssObject {
    pub description: &'static str,
    pub optional: Option<bool>,
    pub additional_properties: Option<bool>,
    pub properties: Box<HashMap<&'static str, Jss>>,
}

#[derive(Debug)]
pub enum Jss {
    Null,
    Boolean(JssBoolean),
    Integer(JssInteger),
    String(JssString),
    Object(JssObject),
    Array(JssArray),
}

pub const DEFAULTBOOL: JssBoolean = JssBoolean {
    description: "",
    optional: None,
    default: None,
};

#[macro_export]
macro_rules! Boolean {
    ($($name:ident => $e:expr),*) => {{
        Jss::Boolean(JssBoolean { $($name: $e, )* ..DEFAULTBOOL})
    }}
}

pub const DEFAULTINTEGER: JssInteger = JssInteger {
    description: "",
    optional: None,
    default: None,
    minimum: None,
    maximum: None,
};

#[macro_export]
macro_rules! Integer {
    ($($name:ident => $e:expr),*) => {{
        Jss::Integer(JssInteger { $($name: $e, )* ..DEFAULTINTEGER})
    }}
}

pub const DEFAULTSTRING: JssString = JssString {
    description: "",
    optional: None,
    default: None,
    min_length: None,
    max_length: None,
};

#[macro_export]
macro_rules! ApiString {
    ($($name:ident => $e:expr),*) => {{
        Jss::String(JssString { $($name: $e, )* ..DEFAULTSTRING})
    }}
}

#[macro_export]
macro_rules! parameter {
    ($($name:ident => $e:expr),*) => {{
        let inner = JssObject {
            description: "",
            optional: None,
            additional_properties: None,
            properties: {
                let mut map = HashMap::<&'static str, Jss>::new();
                $(
                    map.insert(stringify!($name), $e);
                )*
                Box::new(map)
            }
        };

        Jss::Object(inner)
    }}
}



#[test]
fn test_shema1() {
    let schema = Jss::Object(JssObject {
        description: "TEST",
        optional: None,
        additional_properties: None,
        properties: {
            let map = HashMap::new();

            Box::new(map)
        }
    });

    println!("TEST Schema: {:?}", schema);
}

/*
#[test]
fn test_shema1() {
    static PARAMETERS1: PropertyMap = propertymap!{
        force => &Boolean!{
            description => "Test for boolean options."
        },
        text1 => &ApiString!{
            description => "A simple text string.",
            min_length => Some(10),
            max_length => Some(30)
        },
        count => &Integer!{
            description => "A counter for everything.",
            minimum => Some(0),
            maximum => Some(10)
        },
        myarray1 => &Array!{
            description => "Test Array of simple integers.",
            items => &PVE_VMID
        },
        myarray2 => &Jss::Array(JssArray {
            description: "Test Array of simple integers.",
            optional: Some(false),
            items: &Object!{description => "Empty Object."},
        }),
        myobject => &Object!{
            description => "TEST Object.",
            properties => &propertymap!{
                vmid => &PVE_VMID,
                loop => &Integer!{
                    description => "Totally useless thing.",
                    optional => Some(false)
                }
            }
        },
        emptyobject => &Object!{description => "Empty Object."}
    };

    for (k, v) in PARAMETERS1.entries {
        println!("Parameter: {} Value: {:?}", k, v);
    }


}
*/
