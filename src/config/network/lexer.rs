use std::io::BufRead;
use std::iter::Iterator;
use std::collections::{HashMap, VecDeque};

use lazy_static::lazy_static;

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Token {
    Text,
    Comment,
    DHCP,
    Newline,
    Address,
    Auto,
    Gateway,
    Inet,
    Inet6,
    Iface,
    Loopback,
    Manual,
    Netmask,
    Static,
    Attribute,
    EOF,
}

lazy_static! {
    static ref KEYWORDS: HashMap<&'static str, Token> = {
        let mut map = HashMap::new();
        map.insert("address", Token::Address);
        map.insert("auto", Token::Auto);
        map.insert("dhcp", Token::DHCP);
        map.insert("gateway", Token::Gateway);
        map.insert("inet", Token::Inet);
        map.insert("inet6", Token::Inet6);
        map.insert("iface", Token::Iface);
        map.insert("loopback", Token::Loopback);
        map.insert("manual", Token::Manual);
        map.insert("netmask", Token::Netmask);
        map.insert("static", Token::Static);
        map
    };
}

pub struct Lexer<R> {
    input: R,
    eof_count: usize,
    cur_line: Option<VecDeque<(Token, String)>>,
}

impl <R: BufRead> Lexer<R> {

    pub fn new(input: R) -> Self {
        Self { input, eof_count: 0, cur_line: None }
    }

    fn split_line(line: &str) -> VecDeque<(Token, String)> {
        if line.starts_with("#") {
            let mut res = VecDeque::new();
            res.push_back((Token::Comment, line[1..].trim().to_string()));
            return res;
        }
        let mut list: VecDeque<(Token, String)> = line.split_ascii_whitespace().map(|text| {
            let token = KEYWORDS.get(text).unwrap_or(&Token::Text);
            (*token, text.to_string())
        }).collect();

        if line.starts_with(|c: char| c.is_ascii_whitespace() && c != '\n') {
            list.push_front((Token::Attribute, String::from("\t")));
        }
        list
    }
}

impl <R: BufRead> Iterator for Lexer<R> {

    type Item = Result<(Token, String), std::io::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cur_line.is_none() {
            let mut line = String::new();
            match self.input.read_line(&mut line) {
                Err(err) => return Some(Err(err)),
                Ok(0) => {
                    self.eof_count += 1;
                    if self.eof_count == 1 { return Some(Ok((Token::EOF, String::new()))); }
                    return None;
                }
                _ => {}
            }
            self.cur_line = Some(Self::split_line(&line));
        }

        match self.cur_line {
            Some(ref mut  cur_line) => {
                if cur_line.is_empty() {
                    self.cur_line = None;
                    return Some(Ok((Token::Newline, String::from("\n"))));
                } else {
                    let (token, text) = cur_line.pop_front().unwrap();
                    return Some(Ok((token, text)));
                }
            }
            None => {
                return None;
            }
        }
    }
}
