//! Predefined Regular Expressions
//!
//! This is a collection of useful regular expressions

use lazy_static::lazy_static;
use regex::Regex;

#[macro_export]
macro_rules! IPV4OCTET { () => (r"(?:25[0-5]|(?:2[0-4]|1[0-9]|[1-9])?[0-9])") }
#[macro_export]
macro_rules! IPV6H16 { () => (r"(?:[0-9a-fA-F]{1,4})") }
#[macro_export]
macro_rules! IPV6LS32 { () => (concat!(r"(?:(?:", IPV4RE!(), "|", IPV6H16!(), ":", IPV6H16!(), "))" )) }


#[macro_export]
macro_rules! IPV4RE { () => (concat!(r"(?:(?:", IPV4OCTET!(), r"\.){3}", IPV4OCTET!(), ")")) }

#[macro_export]
macro_rules! IPV6RE { () => (concat!(r"(?:",
    r"(?:(?:",                                               r"(?:", IPV6H16!(), r":){6})", IPV6LS32!(), r")|",
    r"(?:(?:",                                             r"::(?:", IPV6H16!(), r":){5})", IPV6LS32!(), r")|",
    r"(?:(?:(?:",                            IPV6H16!(), r")?::(?:", IPV6H16!(), r":){4})", IPV6LS32!(), r")|",
    r"(?:(?:(?:(?:", IPV6H16!(), r":){0,1}", IPV6H16!(), r")?::(?:", IPV6H16!(), r":){3})", IPV6LS32!(), r")|",
    r"(?:(?:(?:(?:", IPV6H16!(), r":){0,2}", IPV6H16!(), r")?::(?:", IPV6H16!(), r":){2})", IPV6LS32!(), r")|",
    r"(?:(?:(?:(?:", IPV6H16!(), r":){0,3}", IPV6H16!(), r")?::(?:", IPV6H16!(), r":){1})", IPV6LS32!(), r")|",
    r"(?:(?:(?:(?:", IPV6H16!(), r":){0,4}", IPV6H16!(), r")?::",                      ")", IPV6LS32!(), r")|",
    r"(?:(?:(?:(?:", IPV6H16!(), r":){0,5}", IPV6H16!(), r")?::",                      ")", IPV6H16!(),  r")|",
    r"(?:(?:(?:(?:", IPV6H16!(), r":){0,6}", IPV6H16!(), r")?::",                                        ")))"))
}

#[macro_export]
macro_rules! IPRE { () => (concat!(r"(?:", IPV4RE!(), "|", IPV6RE!(), ")")) }

lazy_static! {
    pub static ref IP_REGEX: Regex = Regex::new(IPRE!()).unwrap();

    pub static ref SHA256_HEX_REGEX: Regex = Regex::new("^[a-f0-9]{64}$").unwrap();
}
