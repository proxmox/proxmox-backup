use std::sync::Arc;

use super::*;

/// Helper trait implementation for ``rustyline``.
///
/// This can be used to generate interactive commands using
/// ``rustyline`` (readline implementation).
///
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
impl rustyline::Helper for CliHelper {}
