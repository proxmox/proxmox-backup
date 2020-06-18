use anyhow::{bail, Error};

use nom::{
    error::{ParseError, VerboseError},
    bytes::complete::{take_while, take_while1},
    combinator::{map_res, all_consuming, recognize},
    character::complete::{digit1},
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
pub fn multispace0(i: &str)  -> IResult<&str, &str> {
    take_while(|c| c == ' ' || c == '\t')(i)
}

/// Recognizes one or more spaces and tabs (but not carage returns or line feeds)
pub fn multispace1(i: &str)  -> IResult<&str, &str> {
    take_while1(|c| c == ' ' || c == '\t')(i)
}

/// Recognizes one or more non-whitespace-characters
pub fn notspace1(i: &str)  -> IResult<&str, &str> {
    take_while1(|c| !(c == ' ' || c == '\t' || c == '\n'))(i)
}

/// Parse a 64 bit unsigned integer
pub fn parse_u64(i: &str) -> IResult<&str, u64> {
    map_res(recognize(digit1), str::parse)(i)
}

pub fn parse_complete<'a, F, O>(what: &str, i: &'a str, parser: F) -> Result<O, Error>
    where F: Fn(&'a str) -> IResult<&'a str, O>,
{
    match all_consuming(parser)(i) {
        Err(nom::Err::Error(err)) |
        Err(nom::Err::Failure(err)) => {
            bail!("unable to parse {} - {}", what, nom::error::convert_error(i, err));
        }
        Err(err) => {
            bail!("unable to parse {} - {}", what, err);
        }
        Ok((_, data)) => Ok(data),
    }

}
