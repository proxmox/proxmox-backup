use anyhow::{bail, Error};
use hyper::header;

/// Possible Compression Methods, order determines preference (later is preferred)
#[derive(Eq, Ord, PartialEq, PartialOrd, Debug)]
pub enum CompressionMethod {
    Deflate,
    //    Gzip,
    //    Brotli,
}

impl CompressionMethod {
    pub fn content_encoding(&self) -> header::HeaderValue {
        header::HeaderValue::from_static(self.extension())
    }

    pub fn extension(&self) -> &'static str {
        match *self {
            //            CompressionMethod::Brotli => "br",
            //            CompressionMethod::Gzip => "gzip",
            CompressionMethod::Deflate => "deflate",
        }
    }
}

impl std::str::FromStr for CompressionMethod {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            //            "br" => Ok(CompressionMethod::Brotli),
            //            "gzip" => Ok(CompressionMethod::Gzip),
            "deflate" => Ok(CompressionMethod::Deflate),
            // http accept-encoding allows to give weights with ';q='
            other if other.starts_with("deflate;q=") => Ok(CompressionMethod::Deflate),
            _ => bail!("unknown compression format"),
        }
    }
}
