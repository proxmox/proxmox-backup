use std::{
    fs::File,
    io::{stdout, Write},
    panic::{RefUnwindSafe, UnwindSafe},
    path::Path,
};

pub mod api;
pub mod diff;
pub mod inspect;
pub mod recover;

// Returns either a new file, if a path is given, or stdout, if no path is given.
pub(crate) fn outfile_or_stdout<P: AsRef<Path>>(
    path: Option<P>,
) -> std::io::Result<Box<dyn Write + Send + Sync + Unpin + RefUnwindSafe + UnwindSafe>> {
    if let Some(path) = path {
        let f = File::create(path)?;
        Ok(Box::new(f) as Box<_>)
    } else {
        Ok(Box::new(stdout()) as Box<_>)
    }
}
