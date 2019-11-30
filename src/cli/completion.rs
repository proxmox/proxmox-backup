use failure::*;
use std::sync::Arc;

use rustyline::completion::*;

use super::*;

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

pub fn get_completions(
    cmd_def: &CommandLineInterface,
    line: &str,
    skip_first: bool,
) -> (usize, Vec<String>) {

    let (mut args, start ) = match shellword_split_unclosed(line, false) {
        (mut args, None) => {
            args.push("".into());
            (args, line.len())
        }
        (mut args, Some((start , arg, _quote))) => {
            args.push(arg);
            (args, start)
        }
    };

    if skip_first {

        if args.len() == 0 { return (0, Vec::new()); }

        args.remove(0); // no need for program name
    }

    let completions = if !args.is_empty() && args[0] == "help" {
        get_help_completion(cmd_def, &help_command_def(), &args[1..])
    } else {
        get_nested_completion(cmd_def, &args)
    };

    (start, completions)
}

pub struct CliHelper {
    cmd_def: Arc<CommandLineInterface>,
}

impl CliHelper {

    pub fn new(cmd_def: CommandLineInterface) -> Self {
        Self { cmd_def: Arc::new(cmd_def) }
    }

    pub fn cmd_def(&self) -> Arc<CommandLineInterface> {
        self.cmd_def.clone()
    }
}

impl rustyline::completion::Completer for CliHelper {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {

        let line = &line[..pos];

        let (start, completions) = super::get_completions(&*self.cmd_def, line, false);

        return Ok((start, completions));
    }
}

impl rustyline::hint::Hinter for CliHelper {}
impl rustyline::highlight::Highlighter for CliHelper {}

impl rustyline::Helper for CliHelper {

}
