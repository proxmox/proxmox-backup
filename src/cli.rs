//! Tools to create command line parsers
//!
//! This crate provides convenient helpers to create command line
//! parsers using Schema definitions.
//!
//! ## Features
//!
//! - Use declarative API schema to define the CLI
//! - Automatic parameter verification
//! - Automatically generate documentation and manual pages
//! - Automatically generate bash completion helpers
//! - Ability to create interactive commands (using ``rustyline``)
//! - Supports complex/nested commands

mod environment;
pub use environment::*;

mod shellword;
pub use shellword::*;

mod format;
pub use format::*;

mod completion;
pub use completion::*;

mod getopts;
pub use getopts::*;

mod command;
pub use command::*;

mod readline;
pub use readline::*;

use std::collections::HashMap;

use proxmox::api::ApiMethod;

/// Completion function for single parameters.
///
/// Completion functions gets the current parameter value, and should
/// return a list of all possible values.
pub type CompletionFunction = fn(&str, &HashMap<String, String>) -> Vec<String>;

/// Define a simple CLI command.
pub struct CliCommand {
    /// The Schema definition.
    pub info: &'static ApiMethod,
    /// Argument parameter list.
    ///
    /// Those parameters are expected to be passed as command line
    /// arguments in the specified order. All other parameters needs
    /// to be specified as ``--option <value>`` pairs.
    pub arg_param: &'static [&'static str],
    /// Predefined parameters.
    pub fixed_param: HashMap<&'static str, String>,
    /// Completion functions.
    ///
    /// Each parameter may have an associated completion function,
    /// which is called by the shell completion handler.
    pub completion_functions: HashMap<String, CompletionFunction>,
}

impl CliCommand {

    /// Create a new instance.
    pub fn new(info: &'static ApiMethod) -> Self {
        Self {
            info, arg_param: &[],
            fixed_param: HashMap::new(),
            completion_functions: HashMap::new(),
        }
    }

    /// Set argument parameter list.
    pub fn arg_param(mut self, names: &'static [&'static str]) -> Self {
        self.arg_param = names;
        self
    }

    /// Set fixed parameters.
    pub fn fixed_param(mut self, key: &'static str, value: String) -> Self {
        self.fixed_param.insert(key, value);
        self
    }

    /// Set completion functions.
    pub fn completion_cb(mut self, param_name: &str, cb:  CompletionFunction) -> Self {
        self.completion_functions.insert(param_name.into(), cb);
        self
    }
}

/// Define nested CLI commands.
pub struct CliCommandMap {
    /// Each command has an unique name. The map associates names with
    /// command definitions.
    pub commands: HashMap<String, CommandLineInterface>,
}

impl CliCommandMap {

    /// Create a new instance.
    pub fn new() -> Self {
        Self { commands: HashMap:: new() }
    }

    /// Insert another command.
    pub fn insert<S: Into<String>>(mut self, name: S, cli: CommandLineInterface) -> Self {
        self.commands.insert(name.into(), cli);
        self
    }

    /// Insert the help command.
    pub fn insert_help(mut self) -> Self {
        self.commands.insert(String::from("help"), help_command_def().into());
        self
    }

    fn find_command(&self, name: &str) -> Option<(String, &CommandLineInterface)> {

        if let Some(sub_cmd) = self.commands.get(name) {
            return Some((name.to_string(), sub_cmd));
        };

        let mut matches: Vec<&str> = vec![];

        for cmd in self.commands.keys() {
            if cmd.starts_with(name) {
                matches.push(cmd); }
        }

        if matches.len() != 1 { return None; }

        if let Some(sub_cmd) = self.commands.get(matches[0]) {
            return Some((matches[0].to_string(), sub_cmd));
        };

        None
    }
}

/// Define Complex command line interfaces.
pub enum CommandLineInterface {
    Simple(CliCommand),
    Nested(CliCommandMap),
}

impl From<CliCommand> for CommandLineInterface {
    fn from(cli_cmd: CliCommand) -> Self {
         CommandLineInterface::Simple(cli_cmd)
    }
}

impl From<CliCommandMap> for CommandLineInterface {
    fn from(list: CliCommandMap) -> Self {
        CommandLineInterface::Nested(list)
    }
}
