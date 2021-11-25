use percent_encoding::{utf8_percent_encode, AsciiSet};

/// This used to be: `SIMPLE_ENCODE_SET` plus space, `"`, `#`, `<`, `>`, backtick, `?`, `{`, `}`
pub const DEFAULT_ENCODE_SET: &AsciiSet = &percent_encoding::CONTROLS // 0..1f and 7e
    // The SIMPLE_ENCODE_SET adds space and anything >= 0x7e (7e itself is already included above)
    .add(0x20)
    .add(0x7f)
    // the DEFAULT_ENCODE_SET added:
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'?')
    .add(b'{')
    .add(b'}');

/// percent encode a url component
pub fn percent_encode_component(comp: &str) -> String {
    utf8_percent_encode(comp, percent_encoding::NON_ALPHANUMERIC).to_string()
}
