use std::io::{BufRead};
use std::iter::{Peekable, Iterator};
use std::collections::HashSet;

use anyhow::{Error, bail, format_err};
use lazy_static::lazy_static;
use regex::Regex;

use super::helper::*;
use super::lexer::*;

use super::{NetworkConfig, NetworkOrderEntry, Interface, NetworkConfigMethod};

pub struct NetworkParser<R: BufRead> {
    input: Peekable<Lexer<R>>,
    line_nr: usize,
}

impl <R: BufRead> NetworkParser<R> {

    pub fn new(reader: R) -> Self {
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
        let cidr = self.next_text()?;

        let (_address, _mask, ipv6) = parse_cidr(&cidr)?;
        if ipv6 {
            interface.set_cidr_v6(cidr)?;
        } else {
            interface.set_cidr_v4(cidr)?;
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
                Token::Netmask => bail!("netmask is deprecated and no longer supported"),
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
                Token::Loopback => config_method = Some(NetworkConfigMethod::Loopback),
                Token::Static => config_method = Some(NetworkConfigMethod::Static),
                Token::Manual => config_method = Some(NetworkConfigMethod::Manual),
                Token::DHCP => config_method = Some(NetworkConfigMethod::DHCP),
                _ => bail!("unknown iface option {}", text),
            }
        }

        let has_attributes = self.peek()? == Token::Attribute;
        let config_method = config_method.unwrap_or(NetworkConfigMethod::Static);

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
            match self.peek()? {
                Token::EOF => {
                    break;
                }
                Token::Newline => {
                    // skip empty lines
                    self.eat(Token::Newline)?;
                }
                Token::Comment => {
                    let (_, text) = self.next()?;
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

        let existing_interfaces = get_network_interfaces()?;

        lazy_static!{
            static ref PHYSICAL_NIC_REGEX: Regex = Regex::new(r"(?:eth\d+|en[^:.]+|ib\d+)").unwrap();
        }

        for (iface, active) in existing_interfaces.iter()  {
            let exists = PHYSICAL_NIC_REGEX.is_match(iface);
            if let Some(interface) = config.interfaces.get_mut(iface) {
                interface.exists = exists;
                interface.active = *active;
            } else if exists { // also add all physical NICs
                let mut interface = Interface::new(iface.clone());
                interface.set_method_v4(NetworkConfigMethod::Manual)?;
                interface.exists = true;
                interface.active = *active;
                config.interfaces.insert(interface.name.clone(), interface);
                config.order.push(NetworkOrderEntry::Iface(iface.to_string()));
            }
        }

        Ok(config)
    }
}
