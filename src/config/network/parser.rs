use std::io::{BufReader};
use std::fs::File;
use std::iter::{Peekable, Iterator};
use std::collections::HashSet;

use anyhow::{Error, bail, format_err};
use lazy_static::lazy_static;
use regex::Regex;

use proxmox::*; // for IP macros

use super::helper::*;
use super::lexer::*;

use super::{NetworkConfig, NetworkOrderEntry, Interface, ConfigMethod};

pub struct NetworkParser {
    input: Peekable<Lexer<BufReader<File>>>,
    line_nr: usize,
}

impl NetworkParser {

    pub fn new(file: File) -> Self {
        let reader = BufReader::new(file);
        let input = Lexer::new(reader).peekable();
        Self { input, line_nr: 1 }
    }

    fn peek(&mut self) -> Result<Token, Error> {
        match self.input.peek() {
            Some(Err(err)) => {
                bail!("input error - {}", err);
            }
            Some(Ok((token, _))) => {
                return Ok(*token);
            }
            None => {
                bail!("got unexpected end of stream (inside peek)");
            }
        }
    }

    fn next(&mut self) -> Result<(Token, String), Error> {
        match self.input.next() {
            Some(Err(err)) => {
                bail!("input error - {}", err);
            }
            Some(Ok((token, text))) => {
                if token == Token::Newline { self.line_nr += 1; }
                return Ok((token, text));
            }
            None => {
                bail!("got unexpected end of stream (inside peek)");
            }
        }
    }

    fn next_text(&mut self) -> Result<String, Error> {
        match self.next()? {
            (Token::Text, text) => Ok(text),
            (unexpected, _) => bail!("got unexpected token {:?} (expecting Text)", unexpected),
        }
    }

    fn eat(&mut self, expected: Token) -> Result<(), Error> {
        let (next, _) = self.next()?;
        if next != expected {
            bail!("expected {:?}, got {:?}", expected, next);
        }
        Ok(())
    }

    fn parse_auto(&mut self, auto_flag: &mut HashSet<String>) -> Result<(), Error> {
        self.eat(Token::Auto)?;

        loop {
            match self.next()? {
                (Token::Text, iface) => {
                    println!("AUTO {}", iface);
                    auto_flag.insert(iface.to_string());
                }
                (Token::Newline, _) => break,
                unexpected => {
                    bail!("expected {:?}, got {:?}", Token::Text, unexpected);
                }
            }

        }

        Ok(())
    }

    fn parse_iface_address(&mut self, interface: &mut Interface) -> Result<(), Error> {
        self.eat(Token::Address)?;
        let address = self.next_text()?;

        lazy_static! {
            pub static ref ADDRESS_V4_REGEX: Regex = Regex::new(
                concat!(r"^(", IPV4RE!(), r")(?:/(\d{1,2}))?$")
            ).unwrap();
           pub static ref ADDRESS_V6_REGEX: Regex = Regex::new(
               concat!(r"^(", IPV6RE!(), r")(?:/(\d{1,2}))?$")
            ).unwrap();
        }

        if let Some(caps) = ADDRESS_V4_REGEX.captures(&address) {
            let address = caps.get(1).unwrap().as_str();
            interface.set_address_v4(address.to_string())?;
            if let Some(mask) = caps.get(2) {
                let mask = u8::from_str_radix(mask.as_str(), 10)?;
                interface.set_netmask_v4(mask)?;
            }
        } else if let Some(caps) = ADDRESS_V6_REGEX.captures(&address) {
            let address = caps.get(1).unwrap().as_str();
            interface.set_address_v6(address.to_string())?;
            if let Some(mask) = caps.get(2) {
                let mask = u8::from_str_radix(mask.as_str(), 10)?;
                interface.set_netmask_v6(mask)?;
            }
        } else {
             bail!("unable to parse IP address");
        }

        self.eat(Token::Newline)?;

        Ok(())
    }

    fn parse_iface_gateway(&mut self, interface: &mut Interface) -> Result<(), Error> {
        self.eat(Token::Gateway)?;
        let gateway = self.next_text()?;

        if proxmox::tools::common_regex::IP_REGEX.is_match(&gateway) {
            if gateway.contains(':') {
                interface.set_gateway_v6(gateway)?;
            } else {
                interface.set_gateway_v4(gateway)?;
            }
        } else {
            bail!("unable to parse gateway address");
        }

        self.eat(Token::Newline)?;

        Ok(())
    }

    fn parse_to_eol(&mut self) -> Result<String, Error> {
        let mut line = String::new();
        loop {
            match self.next()? {
                (Token::Newline, _) => return Ok(line),
                (_, text) => {
                    if !line.is_empty() { line.push(' '); }
                    line.push_str(&text);
                }
            }
        }
    }

    fn parse_iface_addon_attribute(&mut self, interface: &mut Interface) -> Result<(), Error> {
        let option = self.parse_to_eol()?;
        if !option.is_empty() { interface.push_addon_option(option) };
        Ok(())
    }

    fn parse_iface_netmask(&mut self, interface: &mut Interface) -> Result<(), Error> {
        self.eat(Token::Netmask)?;
        let netmask = self.next_text()?;

        if let Some(mask) = IPV4_MASK_HASH_LOCALNET.get(netmask.as_str())  {
            interface.set_netmask_v4(*mask)?;
        } else {
            match u8::from_str_radix(netmask.as_str(), 10) {
                Ok(mask) => {
                    if mask <= 32 { interface.set_netmask_v4(mask)?; }
                    interface.set_netmask_v6(mask)?;
                }
                Err(err) => {
                    bail!("unable to parse netmask '{}' - {}", netmask, err);
                }
            }
        }

        self.eat(Token::Newline)?;

        Ok(())
    }

    fn parse_iface_attributes(&mut self, interface: &mut Interface) -> Result<(), Error> {

        loop {
            match self.peek()? {
                Token::Attribute => self.eat(Token::Attribute)?,
                Token::Newline => break,
                unexpected => bail!("unknown token {:?}", unexpected),
            }

            match self.peek()? {
                Token::Address => self.parse_iface_address(interface)?,
                Token::Gateway => self.parse_iface_gateway(interface)?,
                Token::Netmask => self.parse_iface_netmask(interface)?,
                _ => {
                    self.parse_iface_addon_attribute(interface)?;
                },
            }
        }

        Ok(())
    }

    fn parse_iface(&mut self, config: &mut NetworkConfig) -> Result<(), Error> {
        self.eat(Token::Iface)?;
        let iface = self.next_text()?;

        let mut address_family_v4 = false;
        let mut address_family_v6 = false;
        let mut config_method = None;

        loop {
            let (token, text) = self.next()?;
            match token {
                Token::Newline => break,
                Token::Inet => address_family_v4 = true,
                Token::Inet6 => address_family_v6 = true,
                Token::Loopback => config_method = Some(ConfigMethod::Loopback),
                Token::Static => config_method = Some(ConfigMethod::Static),
                Token::Manual => config_method = Some(ConfigMethod::Manual),
                Token::DHCP => config_method = Some(ConfigMethod::DHCP),
                _ => bail!("unknown iface option {}", text),
            }
        }

        let has_attributes = self.peek()? == Token::Attribute;
        let config_method = config_method.unwrap_or(ConfigMethod::Static);

        if !(address_family_v4 || address_family_v6) {
            address_family_v4 = true;
            address_family_v6 = true;
        }

        if let Some(mut interface) = config.interfaces.get_mut(&iface) {
            if address_family_v4 {
                interface.set_method_v4(config_method)?;
            }
            if address_family_v6 {
                interface.set_method_v6(config_method)?;
            }

            if has_attributes { self.parse_iface_attributes(&mut interface)?; }
        } else {
            let mut interface = Interface::new(iface.clone());
            if address_family_v4 {
                interface.set_method_v4(config_method)?;
            }
            if address_family_v6 {
                interface.set_method_v6(config_method)?;
            }

            if has_attributes { self.parse_iface_attributes(&mut interface)?; }

            config.interfaces.insert(interface.name.clone(), interface);

            config.order.push(NetworkOrderEntry::Iface(iface));
        }

        Ok(())
    }

    pub fn parse_interfaces(&mut self) -> Result<NetworkConfig, Error> {
        self._parse_interfaces()
            .map_err(|err| format_err!("line {}: {}", self.line_nr, err))
    }

    pub fn _parse_interfaces(&mut self) -> Result<NetworkConfig, Error> {
        let mut config = NetworkConfig::new();

        let mut auto_flag: HashSet<String> = HashSet::new();

        loop {
            let peek = self.peek()?;
            println!("TOKEN: {:?}", peek);
            match peek {
                Token::EOF => {
                    // fixme: trailing comments
                    break;
                }
                Token::Newline => {
                    self.eat(Token::Newline)?;
                    // fixme end of entry
                }
                Token::Comment => {
                    let (_, text) = self.next()?;
                    println!("COMMENT: {}", text);
                    config.order.push(NetworkOrderEntry::Comment(text));
                    self.eat(Token::Newline)?;
                }
                Token::Auto => {
                    self.parse_auto(&mut auto_flag)?;
                }
                Token::Iface => {
                    self.parse_iface(&mut config)?;
                }
                _ => {
                    let option = self.parse_to_eol()?;
                    if !option.is_empty() {
                        config.order.push(NetworkOrderEntry::Option(option));
                    }
                }
            }
        }

        for iface in auto_flag.iter() {
            if let Some(interface) = config.interfaces.get_mut(iface) {
                interface.autostart = true;
            }
        }

        Ok(config)
    }
}
