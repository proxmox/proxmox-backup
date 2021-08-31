use std::fs::File;
use std::io::{self, stdout, Write};
use std::path::Path;

/// Returns either a new file, if a path is given, or stdout, if no path is given.
pub fn outfile_or_stdout<P: AsRef<Path>>(path: Option<P>) -> io::Result<Box<dyn Write>> {
    if let Some(path) = path {
        let f = File::create(path)?;
        Ok(Box::new(f) as Box<dyn Write>)
    } else {
        Ok(Box::new(stdout()) as Box<dyn Write>)
    }
}
