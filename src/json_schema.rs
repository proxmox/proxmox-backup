use crate::static_map::StaticMap;

pub type PropertyMap<'a> = StaticMap<'a, &'a str, &'a Jss<'a>>;

#[derive(Debug)]
pub struct JssBoolean<'a> {
    pub description: &'a str,
    pub optional: Option<bool>,
    pub default: Option<bool>,
}

#[derive(Debug)]
pub struct JssInteger<'a> {
    pub description: &'a str,
    pub optional: Option<bool>,
    pub minimum: Option<usize>,
    pub maximum: Option<usize>,
    pub default: Option<usize>,
}

#[derive(Debug)]
pub struct JssString<'a> {
    pub description: &'a str,
    pub optional: Option<bool>,
    pub default: Option<&'a str>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
}

#[derive(Debug)]
pub struct JssArray<'a> {
    pub description: &'a str,
    pub optional: Option<bool>,
    pub items: &'a Jss<'a>,
}

#[derive(Debug)]
pub struct JssObject<'a> {
    pub description: &'a str,
    pub optional: Option<bool>,
    pub additional_properties: Option<bool>,
    pub properties: &'a PropertyMap<'a>,
}

#[derive(Debug)]
pub enum Jss<'a> {
    Null,
    Boolean(JssBoolean<'a>),
    Integer(JssInteger<'a>),
    String(JssString<'a>),
    Object(JssObject<'a>),
    Array(JssArray<'a>),
}

pub static DEFAULTBOOL: JssBoolean = JssBoolean {
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

pub static DEFAULTINTEGER: JssInteger = JssInteger {
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

pub static DEFAULTSTRING: JssString = JssString {
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

pub static DEFAULTARRAY: JssArray = JssArray {
    description: "",
    optional: None,
    items: &Jss::Null, // is this a reasonable default??
};

#[macro_export]
macro_rules! Array {
    ($($name:ident => $e:expr),*) => {{
        Jss::Array(JssArray { $($name: $e, )* ..DEFAULTARRAY})
    }}
}

pub static EMPTYOBJECT: PropertyMap = PropertyMap { entries: &[] };

pub static DEFAULTOBJECT: JssObject = JssObject {
    description: "",
    optional: None,
    additional_properties: None,
    properties: &EMPTYOBJECT, // is this a reasonable default??
};

#[macro_export]
macro_rules! Object {
    ($($name:ident => $e:expr),*) => {{
        Jss::Object(JssObject { $($name: $e, )* ..DEFAULTOBJECT})
    }}
}


// Standard Option Definitions
pub static PVE_VMID: Jss = Integer!{
    description => "The (unique) ID of the VM.",
    minimum => Some(1)
};

#[macro_export]
macro_rules! propertymap {
    ($($name:ident => $e:expr),*) => {
        PropertyMap {
            entries: &[
                $( ( stringify!($name),  $e), )*
            ]
        }
    }
}

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
