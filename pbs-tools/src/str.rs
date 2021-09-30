//! String related utilities.

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
