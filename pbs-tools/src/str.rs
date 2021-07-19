//! String related utilities.

use std::borrow::Borrow;

pub fn join<S: Borrow<str>>(data: &[S], sep: char) -> String {
    let mut list = String::new();

    for item in data {
        if !list.is_empty() {
            list.push(sep);
        }
        list.push_str(item.borrow());
    }

    list
}

pub fn strip_ascii_whitespace(line: &[u8]) -> &[u8] {
    let line = match line.iter().position(|&b| !b.is_ascii_whitespace()) {
        Some(n) => &line[n..],
        None => return &[],
    };
    match line.iter().rev().position(|&b| !b.is_ascii_whitespace()) {
        Some(n) => &line[..(line.len() - n)],
        None => &[],
    }
}
