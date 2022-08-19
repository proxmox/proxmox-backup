use anyhow::{bail, Error};

use nom::{
    bytes::complete::{take_while, take_while1},
    character::complete::digit1,
    combinator::{all_consuming, map_res, recognize},
    error::{ContextError, VerboseError},
};

pub type IResult<I, O, E = VerboseError<I>> = Result<(I, O), nom::Err<E>>;

pub fn parse_error<'a>(i: &'a str, context: &'static str) -> nom::Err<VerboseError<&'a str>> {
    let err = VerboseError { errors: Vec::new() };
    let err = VerboseError::add_context(i, context, err);
    nom::Err::Error(err)
}

pub fn parse_failure<'a>(i: &'a str, context: &'static str) -> nom::Err<VerboseError<&'a str>> {
    let err = VerboseError { errors: Vec::new() };
    let err = VerboseError::add_context(i, context, err);
    nom::Err::Failure(err)
}

/// Recognizes zero or more spaces and tabs (but not carage returns or line feeds)
pub fn multispace0(i: &str) -> IResult<&str, &str> {
    take_while(|c| c == ' ' || c == '\t')(i)
}

/// Recognizes one or more spaces and tabs (but not carage returns or line feeds)
pub fn multispace1(i: &str) -> IResult<&str, &str> {
    take_while1(|c| c == ' ' || c == '\t')(i)
}

/// Recognizes one or more non-whitespace-characters
pub fn notspace1(i: &str) -> IResult<&str, &str> {
    take_while1(|c| !(c == ' ' || c == '\t' || c == '\n'))(i)
}

/// Parse a 64 bit unsigned integer
pub fn parse_u64(i: &str) -> IResult<&str, u64> {
    map_res(recognize(digit1), str::parse)(i)
}

/// Parse complete input, generate verbose error message with line numbers
pub fn parse_complete<'a, F, O>(what: &str, i: &'a str, parser: F) -> Result<O, Error>
where
    F: FnMut(&'a str) -> IResult<&'a str, O>,
{
    match all_consuming(parser)(i) {
        Err(nom::Err::Error(err)) | Err(nom::Err::Failure(err)) => {
            bail!(
                "unable to parse {} - {}",
                what,
                nom::error::convert_error(i, err)
            );
        }
        Err(err) => {
            bail!("unable to parse {} - {}", what, err);
        }
        Ok((_, data)) => Ok(data),
    }
}

/// Parse complete input, generate simple error message (use this for sinple line input).
pub fn parse_complete_line<'a, F, O>(what: &str, i: &'a str, parser: F) -> Result<O, Error>
where
    F: Fn(&'a str) -> IResult<&'a str, O>,
{
    match all_consuming(parser)(i) {
        Err(nom::Err::Error(VerboseError { errors }))
        | Err(nom::Err::Failure(VerboseError { errors })) => {
            if errors.is_empty() {
                bail!("unable to parse {}", what);
            } else {
                bail!(
                    "unable to parse {} at '{}' - {:?}",
                    what,
                    errors[0].0,
                    errors[0].1
                );
            }
        }
        Err(err) => {
            bail!("unable to parse {} - {}", what, err);
        }
        Ok((_, data)) => Ok(data),
    }
}
