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

