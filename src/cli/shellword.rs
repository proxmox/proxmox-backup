use failure::*;
use rustyline::completion::Quote;

#[derive(PartialEq)]
enum ParseMode {
    Space,
    DoubleQuote,
    EscapeNormal,
    EscapeInDoubleQuote,
    Normal,
    SingleQuote,
}

/// Parsing strings as they would be interpreted by the UNIX Bourne shell.
///
/// - ``finalize``: assume this is a complete command line. Set this
///   to false for the 'completion' helper, which needs to get
///   information about the last unfinished parameter.
///
/// Returns the list of fully parsed words (unescaped and quotes
/// removed). If there are unclosed quotes, the start of that
/// parameter, the parameter value (unescaped and quotes removed), and
/// the quote type are returned.
pub fn shellword_split_unclosed(s: &str, finalize: bool) -> (Vec<String>, Option<(usize, String, Quote)>) {

    let char_indices = s.char_indices();
    let mut args: Vec<String> = Vec::new();
    let mut field_start = None;
    let mut field = String::new();
    let mut mode = ParseMode::Space;

    let space_chars = [' ', '\t', '\n'];

    for (index, c) in char_indices {
        match mode {
            ParseMode::Space => match c {
                '"' => {
                    mode = ParseMode::DoubleQuote;
                    field_start = Some((index, Quote::Double));
                }
                '\\' => {
                    mode = ParseMode::EscapeNormal;
                    field_start = Some((index, Quote::None));
                }
                '\'' => {
                    mode = ParseMode::SingleQuote;
                    field_start = Some((index, Quote::Single));
                }
                c if space_chars.contains(&c) => (), // skip space
                c => {
                    mode = ParseMode::Normal;
                    field_start = Some((index, Quote::None));
                    field.push(c);
                }
            }
            ParseMode::EscapeNormal => {
                mode = ParseMode::Normal;
                field.push(c);
            }
            ParseMode::EscapeInDoubleQuote => {
                // Within double quoted strings, backslashes are only
                // treated as metacharacters when followed by one of
                // the following characters: $ ' " \ newline
                match c {
                    '$' | '\'' | '"' | '\\' | '\n' => (),
                    _ => field.push('\\'),
                }
                field.push(c);
                mode = ParseMode::DoubleQuote;
            }
            ParseMode::Normal => match c {
                '"' => mode = ParseMode::DoubleQuote,
                '\'' => mode = ParseMode::SingleQuote,
                '\\' => mode = ParseMode::EscapeNormal,
                c if space_chars.contains(&c) => {
                    mode = ParseMode::Space;
                    let (_start, _quote) = field_start.take().unwrap();
                    args.push(field.split_off(0));
                }
                c => field.push(c), // continue
            }
            ParseMode::DoubleQuote => match c {
                '"' => mode = ParseMode::Normal,
                '\\' => mode = ParseMode::EscapeInDoubleQuote,
                c => field.push(c), // continue
            }
            ParseMode::SingleQuote => match c {
                // Note: no escape in single quotes
                '\'' => mode = ParseMode::Normal,
                c => field.push(c), // continue
            }
        }
    }

    if finalize && mode == ParseMode::Normal {
        let (_start, _quote) = field_start.take().unwrap();
        args.push(field.split_off(0));
    }

    match field_start {
        Some ((start, quote)) => {
            (args, Some((start, field, quote)))
        }
        None => {
            (args, None)
        }
    }
}

/// Splits a string into a vector of words in the same way the UNIX Bourne shell does.
///
/// Return words unescaped and without quotes.
pub fn shellword_split(s: &str) -> Result<Vec<String>, Error> {

    let (args, unclosed_field) = shellword_split_unclosed(s, true);
    if !unclosed_field.is_none() {
        bail!("shellword split failed - found unclosed quote.");
    }
    Ok(args)
}

#[test]
fn test_shellword_split() {

    let expect = [ "ls", "/etc" ];
    let expect: Vec<String> = expect.iter().map(|v| v.to_string()).collect();

    assert_eq!(expect, shellword_split("ls /etc").unwrap());
    assert_eq!(expect, shellword_split("ls \"/etc\"").unwrap());
    assert_eq!(expect, shellword_split("ls '/etc'").unwrap());
    assert_eq!(expect, shellword_split("ls '/etc'").unwrap());

    assert_eq!(expect, shellword_split("ls /e\"t\"c").unwrap());
    assert_eq!(expect, shellword_split("ls /e'tc'").unwrap());
    assert_eq!(expect, shellword_split("ls /e't''c'").unwrap());

    let expect = [ "ls", "/etc 08x" ];
    let expect: Vec<String> = expect.iter().map(|v| v.to_string()).collect();
    assert_eq!(expect, shellword_split("ls /etc\\ \\08x").unwrap());

    let expect = [ "ls", "/etc \\08x" ];
    let expect: Vec<String> = expect.iter().map(|v| v.to_string()).collect();
    assert_eq!(expect, shellword_split("ls \"/etc \\08x\"").unwrap());
}

#[test]
fn test_shellword_split_unclosed() {

    let expect = [ "ls".to_string() ].to_vec();
    assert_eq!(
        (expect, Some((3, "./File1 name with spaces".to_string(), Quote::Single))),
        shellword_split_unclosed("ls './File1 name with spaces", false)
    );

    let expect = [ "ls".to_string() ].to_vec();
    assert_eq!(
        (expect, Some((3, "./File2 name with spaces".to_string(), Quote::Double))),
        shellword_split_unclosed("ls \"./File2 \"name\" with spaces", false)
    );
}
