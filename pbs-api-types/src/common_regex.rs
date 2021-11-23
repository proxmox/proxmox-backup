//! Predefined Regular Expressions
//!
//! This is a collection of useful regular expressions

use lazy_static::lazy_static;
use regex::Regex;

#[rustfmt::skip]
#[macro_export]
macro_rules! IPV4OCTET { () => (r"(?:25[0-5]|(?:2[0-4]|1[0-9]|[1-9])?[0-9])") }
#[rustfmt::skip]
#[macro_export]
macro_rules! IPV6H16 { () => (r"(?:[0-9a-fA-F]{1,4})") }
#[rustfmt::skip]
#[macro_export]
macro_rules! IPV6LS32 { () => (concat!(r"(?:(?:", IPV4RE!(), "|", IPV6H16!(), ":", IPV6H16!(), "))" )) }

/// Returns the regular expression string to match IPv4 addresses
#[rustfmt::skip]
#[macro_export]
macro_rules! IPV4RE { () => (concat!(r"(?:(?:", IPV4OCTET!(), r"\.){3}", IPV4OCTET!(), ")")) }

/// Returns the regular expression string to match IPv6 addresses
#[rustfmt::skip]
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

/// Returns the regular expression string to match IP addresses (v4 or v6)
#[rustfmt::skip]
#[macro_export]
macro_rules! IPRE { () => (concat!(r"(?:", IPV4RE!(), "|", IPV6RE!(), ")")) }

/// Regular expression string to match IP addresses where IPv6 addresses require brackets around
/// them, while for IPv4 they are forbidden.
#[rustfmt::skip]
#[macro_export]
macro_rules! IPRE_BRACKET { () => (
    concat!(r"(?:",
        IPV4RE!(),
        r"|\[(?:",
            IPV6RE!(),
        r")\]",
    r")"))
}

lazy_static! {
    pub static ref IP_REGEX: Regex = Regex::new(concat!(r"^", IPRE!(), r"$")).unwrap();
    pub static ref IP_BRACKET_REGEX: Regex =
        Regex::new(concat!(r"^", IPRE_BRACKET!(), r"$")).unwrap();
    pub static ref SHA256_HEX_REGEX: Regex = Regex::new(r"^[a-f0-9]{64}$").unwrap();
    pub static ref SYSTEMD_DATETIME_REGEX: Regex =
        Regex::new(r"^\d{4}-\d{2}-\d{2}( \d{2}:\d{2}(:\d{2})?)?$").unwrap();
}

#[test]
fn test_regexes() {
    assert!(IP_REGEX.is_match("127.0.0.1"));
    assert!(IP_REGEX.is_match("::1"));
    assert!(IP_REGEX.is_match("2014:b3a::27"));
    assert!(IP_REGEX.is_match("2014:b3a::192.168.0.1"));
    assert!(IP_REGEX.is_match("2014:b3a:0102:adf1:1234:4321:4afA:BCDF"));

    assert!(IP_BRACKET_REGEX.is_match("127.0.0.1"));
    assert!(IP_BRACKET_REGEX.is_match("[::1]"));
    assert!(IP_BRACKET_REGEX.is_match("[2014:b3a::27]"));
    assert!(IP_BRACKET_REGEX.is_match("[2014:b3a::192.168.0.1]"));
    assert!(IP_BRACKET_REGEX.is_match("[2014:b3a:0102:adf1:1234:4321:4afA:BCDF]"));
}
