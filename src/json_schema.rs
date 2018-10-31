pub type StaticPropertyMap = phf::Map<&'static str, Jss>;

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
    pub items: &'static Jss,
}

#[derive(Debug)]
pub struct JssObject {
    pub description: &'static str,
    pub optional: Option<bool>,
    pub additional_properties: Option<bool>,
    pub properties: &'static StaticPropertyMap,
}

#[derive(Debug)]
pub enum Jss {
    Null,
    Boolean(JssBoolean),
    Integer(JssInteger),
    String(JssString),
    Object(JssObject),
    Array(JssArray),
    Reference { reference: &'static Jss },
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

pub static EMPTYOBJECT: StaticPropertyMap = phf_map!{};

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

